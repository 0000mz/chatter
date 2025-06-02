use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TwitchAuthPayload {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(with = "serde_millis")]
    pub expiration_timestamp: std::time::Instant,
}


#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub twitch_auth: Option<TwitchAuthPayload>,
}

#[derive(Deserialize, Clone)]
pub struct ApiConfig {
    pub twitch_api_client_id: String,
    pub twitch_api_secret: String,
}

pub struct AppData;

impl AppData {
    fn get_directory() -> String {
        let home_path = std::env::var("HOME").unwrap();
        format!("{}/.streamchat", home_path)
    }

    fn get_api_config_filepath() -> String {
        format!("{}/api.toml", Self::get_directory())
    }

    fn get_app_config_filepath() -> String {
        format!("{}/config.toml", Self::get_directory())
    }

    pub async fn get_configs() -> (ApiConfig, AppConfig) {
        let config_path = AppData::get_app_config_filepath();
        let config_contents = std::fs::read_to_string(&config_path).unwrap();
        let app_config: AppConfig = toml::from_str(config_contents.as_str()).unwrap();

        let api_config_path = AppData::get_api_config_filepath();
        let api_config_contents = std::fs::read_to_string(&api_config_path).unwrap();
        let api_config: ApiConfig = toml::from_str(api_config_contents.as_str()).unwrap();

        (api_config, app_config)
    }

    pub async fn setup() -> Result<Option<(ApiConfig, AppConfig)>, reqwest::Error> {
        let appdata_path = AppData::get_directory();
        if !std::path::Path::new(appdata_path.as_str()).exists() {
            println!("Creating appdata directory: {}", appdata_path);
            std::fs::create_dir(&appdata_path).unwrap();
        }

        let api_config_path = AppData::get_api_config_filepath();
        if !std::path::Path::new(api_config_path.as_str()).exists() {
            // TODO: Give better instructions as to how the `api.toml` file should be structured.
            eprintln!(
                "No api config file found. Create it and store the twitch client id/secret there: {}",
                api_config_path,
            );
            return Ok(None);
        }

        let api_config_contents = std::fs::read_to_string(&api_config_path).unwrap();
        let api_config: ApiConfig = toml::from_str(api_config_contents.as_str()).unwrap();

        let config_path = AppData::get_app_config_filepath();
        if !std::path::Path::new(config_path.as_str()).exists() {
            println!("Creating config file...");
            std::fs::File::create(config_path.as_str()).unwrap();
        }

        let config_contents = std::fs::read_to_string(&config_path).unwrap();
        let mut app_config: AppConfig = toml::from_str(config_contents.as_str()).unwrap();

        if let None = app_config.twitch_auth {
            println!("No twitch user access token in app config; regenerating...");
            match auth_twitch_new_access_token(
                api_config.twitch_api_client_id.as_str(),
                api_config.twitch_api_secret.as_str(),
            )
            .await?
            {
                Some(twitch_auth_info) => {
                    app_config.twitch_auth = Some(twitch_auth_info);
                }
                None => {}
            }
        } else if let Some(twitch_auth) = app_config.twitch_auth.as_ref()
            && std::time::Instant::now() >= twitch_auth.expiration_timestamp
        {
            println!("Using refresh token to generate new access token.");
            match auth_twitch_refresh_access_token(
                twitch_auth.refresh_token.as_str(),
                api_config.twitch_api_client_id.as_str(),
                api_config.twitch_api_secret.as_str(),
            )
            .await?
            {
                Some(twitch_auth_info) => {
                    app_config.twitch_auth = Some(twitch_auth_info);
                }
                None => {
                    eprintln!("Failed to use refresh token to regenerate access token...");
                    // TODO: Display some error to the user...
                }
            }
        } else {
            println!("Using cached twitch user access token.");
        }

        println!("Updating app config.");
        let updated_app_config_str = toml::to_string(&app_config).unwrap();
        std::fs::write(config_path, updated_app_config_str).unwrap();

        Ok(Some(AppData::get_configs().await))
    }
}

// Refreshes an access token using the `refresh_token`.
async fn auth_twitch_refresh_access_token(
    refresh_token: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<Option<TwitchAuthPayload>, reqwest::Error> {
    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ];
    let res = client
        .post("https://id.twitch.tv/oauth2/token")
        .form(&params)
        .send()
        .await?;

    if res.status() != reqwest::StatusCode::OK {
        eprintln!("Twitch refresh token response: status={}", res.status());
        return Ok(None);
    }
    let payload = res.text().await?;
    Ok(twitch_parse_access_token_response(payload.as_str()).await)
}

async fn auth_twitch_new_access_token(
    twitch_client_id: &str,
    twitch_secret: &str,
) -> Result<Option<TwitchAuthPayload>, reqwest::Error> {
    // Start a server to handle the twitch auth path.
    let secret_code = 204;
    let port = 7890;
    let (tx, rx) = std::sync::mpsc::channel();
    let (exit_send, exit_recv) = std::sync::mpsc::channel();

    let state = uuid::Uuid::new_v4();
    let server = rouille::Server::new(format!("0.0.0.0:{}", port), move |request| {
        rouille::log(request, std::io::stdout(), || {
            rouille::router!(request,
              (GET) (/ping) => {
                println!("/ping received. Sending pong...");
                rouille::Response::text(format!("PONG {}", secret_code))
              },
              (GET) (/twitch_auth_callback) => {
                match request.get_param("state") {
                  None => {
                    rouille::Response::text("Failed to acquire state.")
                  },
                  Some(returned_state) => {
                    if returned_state != state.to_string() {
                      rouille::Response::text(format!("Invalid state received... expected={} actual={}", state, returned_state))
                    } else {
                      match request.get_param("code") {
                        None => {
                          rouille::Response::text("Failed to authenticate user...")
                        },
                        Some(code) => {
                          tx.send(code).unwrap();
                          rouille::Response::html("Auth successful, return to app.")
                        }
                      }
                    }
                  }
                }
              },
              _ => rouille::Response::empty_404()
            )
        })
    })
    .unwrap();
    let (server_handle, server_stopper) = server.stoppable();

    tokio::spawn(async move {
        println!("Spinning up local server to get callback for oauth request...");
        server_handle.join().unwrap();
        println!("HTTP server join succeeded. Exiting http listen thread.");
        exit_send.send(true).unwrap();
    });

    // Send a request to the /ping endpoing of the temporary http server that was just
    // spun up so that we can ensure that the server is ready to accept requests.
    println!("Sending secret code to server and waiting for response.");
    let server_url = format!("http://localhost:{}", port);
    let client = reqwest::Client::new();
    let response = client.get(format!("{}/ping", server_url)).send().await?;
    if response.status() != reqwest::StatusCode::OK {
        println!(
            "Server ping check failed... status code={}",
            response.status()
        );
        return Ok(None);
    }
    println!("Checking body of /ping response.");
    let ping_body = response.text().await?;
    if ping_body != format!("PONG {}", secret_code) {
        println!("Invalid secret code when analyzing ping response.");
        return Ok(None);
    }

    // Send a request to twitch's user authentication endpoint that will redirect to the
    // temporary http server that will extract the twitch user's auth code.
    println!("Sending twitch auth request.");
    let redirect_uri = format!("{}/twitch_auth_callback", server_url);
    let scopes = ["user:read:chat", "user:write:chat"];
    let scope_urlencoded = (|| {
        let scopes_joined = scopes.join(" ");
        urlencoding::encode(scopes_joined.as_str()).into_owned()
    })();
    let twitch_uri = format!(
        "https://id.twitch.tv/oauth2/authorize\
        ?response_type=code\
        &client_id={}\
        &redirect_uri={}\
        &scope={}\
        &state={}",
        twitch_client_id, redirect_uri, scope_urlencoded, state
    );
    // open "twitch_url" in browser... the server callback will handle the collection
    // of the access token.
    println!("Twitch callback url: {}", redirect_uri);
    println!("Opening url: {}", twitch_uri);
    match open::that(twitch_uri) {
        Ok(_) => {
            println!("Opened twitch url... waiting for callback...");
        }
        Err(_) => {
            eprintln!("Error opening twitch url..");
            return Ok(None);
        }
    }
    let authorization_code = rx.recv().unwrap();
    println!("Received authorization token: {}", authorization_code);

    // Send shutdown code to the server.
    // TODO: This will only stop the server in the happy-path. The server shutdown need to happen when
    // errors occur too...
    server_stopper.send(()).unwrap();
    exit_recv.recv().unwrap(); // Ensure that the http server has exited.

    // Use the authorization token to get the user token and referesh token...
    println!("Using authorization code to get user's access and referch tokens.");
    let user_token_params = [
        ("client_id", twitch_client_id),
        ("client_secret", twitch_secret),
        ("code", authorization_code.as_str()),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri.as_str()),
    ];
    let client = reqwest::Client::new();
    let res = client
        .post("https://id.twitch.tv/oauth2/token")
        .form(&user_token_params)
        .send()
        .await?;

    if res.status() != reqwest::StatusCode::OK {
        println!(
            "Invalid status code when trading auth token for user token: {}",
            res.status()
        );
        return Ok(None);
    }
    let payload = res.text().await?;
    Ok(twitch_parse_access_token_response(payload.as_str()).await)
}

async fn twitch_parse_access_token_response(response_payload: &str) -> Option<TwitchAuthPayload> {
    #[derive(Deserialize)]
    struct InlineTwitchAuthPayload {
        access_token: String,
        refresh_token: String,
        expires_in: u64,
    }

    let twitch_user_pload: InlineTwitchAuthPayload =
        serde_json::from_str(response_payload).unwrap();

    if twitch_user_pload.access_token.len() == 0 || twitch_user_pload.refresh_token.len() == 0 {
        eprintln!("Failed to get user's access or refresh token...");
        return None;
    }
    let full_twitch_user_payload = TwitchAuthPayload {
        access_token: twitch_user_pload.access_token,
        refresh_token: twitch_user_pload.refresh_token,
        expiration_timestamp: std::time::Instant::now()
            .checked_add(std::time::Duration::from_secs(twitch_user_pload.expires_in))
            .unwrap(),
    };
    println!("RESULT={:?}", full_twitch_user_payload);
    Some(full_twitch_user_payload)
}
