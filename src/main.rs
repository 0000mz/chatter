use iced::widget::{button, column, rich_text, span};
use iced::{Font, color, font};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::vec::Vec;

#[tokio::main]
async fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        panic!("Expected argument to chat stream.");
    }
    iced::application("Stream Chat", StreamChat::update, StreamChat::view).run_with(move || {
        (
            StreamChat::new(args[1].clone()),
            iced::Task::done(Message::InitializeChat),
        )
    })
}

#[derive(Debug, Clone)]
enum Message {
    InitializeChat,
    InitializeChatError(String),
    MessageStreamListen,
    AuthTwitch,
    AuthTwitchResult(bool),
    // Attach the chat stream to the streamchat instance to receive messages from the channel.
    // The parameter should be the twitch stream name to attach.
    AttachChatStream(/*stream_name*/ String),
}

struct StreamChat {
    stream: Option<Box<dyn MessageStream>>,
    active_messages: VecDeque<UserMessage>,
    stream_chat_url: String,
}

impl StreamChat {
    fn new(message_path: String) -> Self {
        StreamChat {
            stream: Option::None,
            active_messages: VecDeque::new(),
            stream_chat_url: message_path,
        }
    }

    fn create_message_stream(&self) -> std::result::Result<Box<dyn MessageStream>, String> {
        let url_parts = self.stream_chat_url.splitn(2, "://").collect::<Vec<_>>();
        if url_parts.len() != 2 {
            return Err(String::from("Invalid url, no protocol found."));
        }

        let (protocol, path) = (url_parts[0], url_parts[1]);
        match protocol {
            "file" => Ok(Box::new(CsvMessageStream::new_from_file(path))),
            _ => Err(String::from(format!("Invalid protocol: {}", protocol))),
        }
    }

    fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::InitializeChat => match self.create_message_stream() {
                Ok(message_stream) => {
                    self.stream = Some(message_stream);
                    iced::Task::done(Message::MessageStreamListen)
                }
                Err(err) => iced::Task::done(Message::InitializeChatError(err)),
            },
            Message::InitializeChatError(error_message) => {
                todo!("Show some error dialog here...: {}", error_message);
                // iced::Task::done(Message::InitializeChatError(error_message))
            }
            Message::MessageStreamListen => {
                let new_messages = self.stream.as_mut().unwrap().collect_messages();
                for message in new_messages {
                    self.active_messages.push_front(message);
                }
                while self.active_messages.len() > 100 {
                    self.active_messages.pop_back();
                }
                iced::Task::done(Message::MessageStreamListen)
            }
            Message::AuthTwitch => iced::Task::perform(auth_twitch(), Message::AuthTwitchResult),
            Message::AuthTwitchResult(success) => {
                if success {
                    iced::Task::done(Message::AttachChatStream(String::from("caedrel")))
                } else {
                    todo!("Handle auth failures here...");
                }
            }
            Message::AttachChatStream(stream_name) => {
                println!("TODO Attach chat stream for channel: {}", stream_name);
                iced::Task::none()
            }
        }
    }

    fn view(&self) -> iced::Element<Message> {
        let mut v = column![button("Auth Twitch").on_press(Message::AuthTwitch)];
        for msg in &self.active_messages {
            v = v.push(
                rich_text![
                    span(format!("{}: ", &msg.username))
                        .color(color!(0xff0000))
                        .font(Font {
                            weight: font::Weight::Bold,
                            ..Font::default()
                        }),
                    span(&msg.message)
                ]
                .size(20),
            );
        }
        v.into()
    }
}

trait MessageStream {
    fn collect_messages(&mut self) -> Vec<UserMessage>;
}

struct CsvMessageStream {
    messages: VecDeque<UserMessage>,
}

impl MessageStream for CsvMessageStream {
    fn collect_messages(&mut self) -> Vec<UserMessage> {
        let mut released_messages = vec![];
        let mut itr = self.messages.iter();
        let mut nb_released = 0;

        while let Some(message) = itr.next() {
            if std::time::Instant::now() >= message.timestamp {
                released_messages.push(message.clone());
                nb_released += 1;
            } else {
                break;
            }
        }

        for _ in 0..nb_released {
            self.messages.pop_front();
        }

        released_messages
    }
}

impl CsvMessageStream {
    fn new_from_file(filepath: &str) -> Self {
        let now = std::time::Instant::now();
        let timestamp_fn = |duration_sec: u64| {
            now.checked_add(std::time::Duration::from_secs(duration_sec))
                .unwrap()
        };

        let mut first_line = true;

        let contents = std::fs::read_to_string(filepath).unwrap();
        let mut user_messages = vec![];
        for line in contents.lines() {
            if first_line {
                first_line = false;
                continue;
            }
            let parts = line.split(',').collect::<Vec<_>>();
            if parts.len() < 3 {
                panic!("Expected each entry to have 3 parts: line=\"{}\"", line);
            }
            let duration_sec = parts[2].parse().expect("Expected number.");
            user_messages.push(UserMessage::new(
                parts[0],
                parts[1],
                timestamp_fn(duration_sec),
            ));
        }
        CsvMessageStream {
            messages: user_messages.into(),
        }
    }
}

#[derive(Clone)]
struct UserMessage {
    username: String,
    message: String,
    timestamp: std::time::Instant,
}

impl UserMessage {
    fn new(username: &str, message: &str, timestamp: std::time::Instant) -> Self {
        UserMessage {
            username: username.into(),
            message: message.into(),
            timestamp,
        }
    }
}

async fn auth_twitch() -> bool {
    match auth_twitch_wrap().await {
        Ok(res) => res,
        Err(_) => false,
    }
}

#[derive(Serialize, Deserialize)]
struct AppConfig {
    twitch_auth: Option<TwitchAuthPayload>,
}

#[derive(Deserialize)]
struct ApiConfig {
    twitch_api_client_id: String,
    twitch_api_secret: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct TwitchAuthPayload {
    access_token: String,
    refresh_token: String,
    #[serde(with = "serde_millis")]
    expiration_timestamp: std::time::Instant,
}

async fn auth_twitch_wrap() -> Result<bool, reqwest::Error> {
    let home_path = std::env::var("HOME").unwrap();
    let appdata_path = format!("{}/.streamchat", home_path);
    if !std::path::Path::new(appdata_path.as_str()).exists() {
        println!("Creating appdata directory: {}", appdata_path);
        std::fs::create_dir(&appdata_path).unwrap();
    }

    let api_config_path = format!("{}/api.toml", appdata_path);
    if !std::path::Path::new(api_config_path.as_str()).exists() {
        // TODO: Give better instructions as to how the `api.toml` file should be structured.
        eprintln!(
            "No api config file found. Create it and store the twitch client id/secret there: {}",
            api_config_path,
        );
        return Ok(false);
    }

    let api_config_contents = std::fs::read_to_string(&api_config_path).unwrap();
    let api_config: ApiConfig = toml::from_str(api_config_contents.as_str()).unwrap();

    let config_path = format!("{}/config.toml", appdata_path);
    if !std::path::Path::new(config_path.as_str()).exists() {
        println!("Creating config file...");
        std::fs::File::create(config_path.as_str()).unwrap();
    }

    let config_contents = std::fs::read_to_string(&config_path).unwrap();
    let mut app_config: AppConfig = toml::from_str(config_contents.as_str()).unwrap();

    // TODO: Also check if the token has expired...
    if let None = app_config.twitch_auth {
        println!("No twitch user access token in app config; regenerating...");
        match auth_twitch_new_access_token(
            api_config.twitch_api_client_id,
            api_config.twitch_api_secret,
        )
        .await?
        {
            Some(twitch_auth_info) => {
                app_config.twitch_auth = Some(twitch_auth_info);
            }
            None => {}
        }
    } else {
        println!("Using cached twitch user access token.");
    }

    println!("Updating app config.");
    let updated_app_config_str = toml::to_string(&app_config).unwrap();
    std::fs::write(config_path, updated_app_config_str).unwrap();

    Ok(true)
}

async fn auth_twitch_new_access_token(
    twitch_client_id: String,
    twitch_secret: String,
) -> Result<Option<TwitchAuthPayload>, reqwest::Error> {
    // Start a server to handle the twitch auth path.
    let secret_code = 204;
    let port = 7890;
    let (tx, rx) = std::sync::mpsc::channel();
    let (exit_send, exit_recv) = std::sync::mpsc::channel();

    let server = rouille::Server::new(format!("0.0.0.0:{}", port), move |request| {
        rouille::log(request, std::io::stdout(), || {
            rouille::router!(request,
              (GET) (/ping) => {
                println!("/ping received. Sending pong...");
                rouille::Response::text(format!("PONG {}", secret_code))
              },
              (GET) (/twitch_auth_callback) => {
                match request.get_param("code") {
                  None => {
                    rouille::Response::text("Failed to authenticate user...")
                  },
                  Some(code) => {
                    // TODO: Also verify state...
                    tx.send(code).unwrap();
                    rouille::Response::html("Auth successful, return to app.")
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
    let state = "placeholder";
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
        ("client_id", twitch_client_id.as_str()),
        ("client_secret", twitch_secret.as_str()),
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

    #[derive(Deserialize)]
    struct InlineTwitchAuthPayload {
        access_token: String,
        refresh_token: String,
        expires_in: u64,
    }
    let twitch_user_pload: InlineTwitchAuthPayload =
        serde_json::from_str(payload.as_str()).unwrap();

    if twitch_user_pload.access_token.len() == 0 || twitch_user_pload.refresh_token.len() == 0 {
        eprintln!("Failed to get user's access or refresh token...");
        return Ok(None);
    }
    let full_twitch_user_payload = TwitchAuthPayload {
        access_token: twitch_user_pload.access_token,
        refresh_token: twitch_user_pload.refresh_token,
        expiration_timestamp: std::time::Instant::now()
            .checked_add(std::time::Duration::from_secs(twitch_user_pload.expires_in))
            .unwrap(),
    };
    println!("RESULT={:?}", full_twitch_user_payload);
    Ok(Some(full_twitch_user_payload))
}
