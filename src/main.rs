use async_trait::async_trait;
use iced::widget::{column, rich_text, span};
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
enum MessageStreamType {
    TwitchInitializer(TwitchMessageStreamInitializer),
    CsvInitializer(CsvMessageStreamInitializer),
    CsvMessageStream(CsvMessageStream),
    TwitchMessageStream(TwitchMessageStream),
    InvalidStream,
}

#[derive(Debug, Clone)]
enum Message {
    InitializeChat,
    InitializeChatError(String),
    AttachMessageStream(MessageStreamType),
    MessageStreamListen,
    Terminate,
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

    fn create_message_stream_initializer(&self) -> std::result::Result<MessageStreamType, String> {
        let url_parts = self.stream_chat_url.splitn(2, "://").collect::<Vec<_>>();
        if url_parts.len() != 2 {
            return Err(String::from("Invalid url, no protocol found."));
        }

        let (protocol, path) = (url_parts[0], url_parts[1]);
        match protocol {
            "file" => Ok(MessageStreamType::CsvInitializer(
                CsvMessageStreamInitializer::new_from_file(path),
            )),
            "twitch" => Ok(MessageStreamType::TwitchInitializer(
                TwitchMessageStreamInitializer::new_for_stream(path),
            )),
            _ => Err(String::from(format!("Invalid protocol: {}", protocol))),
        }
    }

    fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::InitializeChat => match self.create_message_stream_initializer() {
                Ok(MessageStreamType::CsvInitializer(mut initializer)) => iced::Task::perform(
                    (|| async move {
                        let stream = initializer.init_stream().await;
                        if let Some(csv) = (*stream).as_any().downcast_ref::<CsvMessageStream>() {
                            MessageStreamType::CsvMessageStream(csv.clone())
                        } else {
                            MessageStreamType::InvalidStream
                        }
                    })(),
                    Message::AttachMessageStream,
                ),
                Ok(MessageStreamType::TwitchInitializer(mut initializer)) => iced::Task::perform(
                    (|| async move {
                        let stream = initializer.init_stream().await;
                        if let Some(twitch) =
                            (*stream).as_any().downcast_ref::<TwitchMessageStream>()
                        {
                            MessageStreamType::TwitchMessageStream(twitch.clone())
                        } else {
                            MessageStreamType::InvalidStream
                        }
                    })(),
                    Message::AttachMessageStream,
                ),
                Ok(stream_type) => iced::Task::done(Message::InitializeChatError(format!(
                    "Unknown stream type: {:?}",
                    stream_type
                ))),
                Err(err) => iced::Task::done(Message::InitializeChatError(err)),
            },
            Message::AttachMessageStream(stream_type) => {
                if self.stream.is_some() {
                    iced::Task::done(Message::InitializeChatError(String::from(
                        "A stream is already attached; cannot attach another stream.",
                    )))
                } else {
                    match stream_type {
                        MessageStreamType::CsvMessageStream(csv_stream) => {
                            self.stream = Some(Box::new(csv_stream));
                        }
                        MessageStreamType::TwitchMessageStream(twitch_stream) => {
                            self.stream = Some(Box::new(twitch_stream));
                        }
                        _ => {
                            return iced::Task::done(Message::InitializeChatError(format!(
                                "Failed to extract message stream from: {:?}",
                                stream_type
                            )));
                        }
                    }
                    assert!(self.stream.is_some());
                    iced::Task::done(Message::MessageStreamListen)
                }
            }
            Message::InitializeChatError(error_message) => {
                // TODO: display the error in some modal.
                eprintln!("Erorr: {}", error_message);
                iced::Task::done(Message::Terminate)
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
            Message::Terminate => iced::window::get_oldest().then(|id| {
                if let Some(id) = id {
                    iced::window::close(id)
                } else {
                    eprintln!("Failed to terminate -- could not acquire window id.");
                    iced::Task::none()
                }
            }),
        }
    }

    fn view(&self) -> iced::Element<Message> {
        let mut v = column![];
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

// Helper trait to transform a type T to any.
// Useful for downcasting dyn Trait to concrete type dyn T.
pub trait AToAny: 'static {
    fn as_any(&self) -> &dyn std::any::Any;
}
impl<T: 'static> AToAny for T {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait]
trait MessageStream: AToAny {
    fn collect_messages(&mut self) -> Vec<UserMessage>;
}

struct CsvMessageStream {
    messages: VecDeque<UserMessage>,
}

impl std::fmt::Debug for CsvMessageStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "CsvMessageStream <>")
    }
}

impl Clone for CsvMessageStream {
    fn clone(&self) -> Self {
        CsvMessageStream {
            messages: self.messages.clone(),
        }
    }
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

#[async_trait]
trait MessageStreamInitializer {
    async fn init_stream(&mut self) -> Box<dyn MessageStream>;
}

#[derive(Clone)]
struct CsvMessageStreamInitializer {
    messages: VecDeque<UserMessage>,
}

impl std::fmt::Debug for CsvMessageStreamInitializer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(
            f,
            "CsvMessageStreamInitializer <# messages = {}>",
            self.messages.len()
        )
    }
}

#[async_trait]
impl MessageStreamInitializer for CsvMessageStreamInitializer {
    async fn init_stream(&mut self) -> Box<dyn MessageStream> {
        Box::new(CsvMessageStream {
            messages: self.messages.clone(),
        })
    }
}

impl CsvMessageStreamInitializer {
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
        CsvMessageStreamInitializer {
            messages: user_messages.into(),
        }
    }
}

struct TwitchMessageStream {
    stream_name: String,
}

impl std::fmt::Debug for TwitchMessageStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "TwitchMessageStream <channel = {}>", self.stream_name)
    }
}

impl Clone for TwitchMessageStream {
    fn clone(&self) -> Self {
        TwitchMessageStream {
            stream_name: self.stream_name.clone(),
        }
    }
}

impl MessageStream for TwitchMessageStream {
    fn collect_messages(&mut self) -> Vec<UserMessage> {
        eprintln!("TwitchMessageStream::collect_messages: no messages to collet yet...");
        Vec::new()
    }
}

#[derive(Debug, Clone)]
struct TwitchMessageStreamInitializer {
    stream_name: String,
}

#[async_trait]
impl MessageStreamInitializer for TwitchMessageStreamInitializer {
    async fn init_stream(&mut self) -> Box<dyn MessageStream> {
        if let Err(twitch_auth_err) = setup_twitch_oauth().await {
            eprintln!("error: {}", twitch_auth_err);
        }
        Box::new(TwitchMessageStream {
            stream_name: String::from(self.stream_name.clone()),
        })
    }
}

impl TwitchMessageStreamInitializer {
    fn new_for_stream(stream: &str) -> Self {
        TwitchMessageStreamInitializer {
            stream_name: String::from(stream),
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

// Setup the twitch user authentication.
// - Acquires the user's access and refresh token that will be
//   used for interacting with the Twitch API.
// - Stores the token information into the appdata storage.
async fn setup_twitch_oauth() -> Result<bool, reqwest::Error> {
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
    } else {
        println!("Using cached twitch user access token.");
    }

    println!("Updating app config.");
    let updated_app_config_str = toml::to_string(&app_config).unwrap();
    std::fs::write(config_path, updated_app_config_str).unwrap();

    // Subscribe to the EventSub for receiving chat messages.
    println!("Attempting to subscribe to twitch chat event sub.");
    match auth_twitch_chat_event_sub(
        api_config.twitch_api_client_id.as_str(),
        app_config.twitch_auth.unwrap().access_token.as_str(),
    )
    .await
    {
        Ok(success) => {
            println!("auth_twitch_chat_event_sub: success={}", success);
        }
        Err(err) => {
            eprintln!("auth_twitch_chat_event_sub: err={:?}", err);
        }
    }

    Ok(true)
}

async fn auth_twitch_chat_event_sub(
    client_id: &str,
    access_token: &str,
) -> Result<bool, reqwest::Error> {
    let client = reqwest::Client::new();

    let bearer_str = format!("Bearer {}", access_token);

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "Authorization",
        reqwest::header::HeaderValue::from_str(bearer_str.as_str()).unwrap(),
    );
    headers.insert(
        "Client-Id",
        reqwest::header::HeaderValue::from_str(client_id).unwrap(),
    );
    headers.insert(
        "Content-Type",
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    #[derive(Serialize)]
    struct InlineEventSubCondition {
        broadcaster_user_id: String,
        user_id: String,
    }

    #[derive(Serialize)]
    struct InlineTransport {
        method: String,
        session_id: String,
    }

    #[derive(Serialize)]
    struct TwitchEventSubRequestBody {
        #[serde(rename(serialize = "type"))]
        reqtype: String,
        version: String,
        condition: InlineEventSubCondition,
        transport: InlineTransport,
    }

    let body = TwitchEventSubRequestBody {
        reqtype: String::from("channel.chat.message"),
        version: String::from("1"),
        condition: InlineEventSubCondition {
            broadcaster_user_id: String::from("23211159" /* atrioc */),
            // TODO: Do not hardcode the user IDs into the request...
            user_id: String::from("76000742" /* infallible_mob_ */),
        },
        transport: InlineTransport {
            method: String::from("websocket"),
            // TODO: replace session id...
            session_id: String::from("example_session_id"),
        },
    };

    // TODO: More info on how to connect to twitch eventsub:
    // https://discuss.dev.twitch.com/t/eventsub-websockets-subscriptions-failing/41879/2

    let response = client
        .post("https://api.twitch.tv/helix/eventsub/subscriptions")
        .headers(headers)
        .json(&body)
        .send()
        .await?;

    println!("EventSub.channel_messages = Response={}", response.status());
    if response.status() != reqwest::StatusCode::OK {
        eprintln!("Bad response status...");
        if let Ok(response_body) = response.text().await {
            eprintln!("Response body={}", response_body);
        }
        return Ok(false);
    }

    Ok(true)
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
