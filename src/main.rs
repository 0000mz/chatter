use async_trait::async_trait;
use iced::futures::StreamExt;
use iced::widget::{column, rich_text, span};
use iced::{Font, color, font};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::vec::Vec;

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        panic!("Expected argument to chat stream.");
    }
    iced::application(StreamChat::boot, StreamChat::update, StreamChat::view)
        .subscription(StreamChat::subscription)
        .run()
}

#[derive(Clone)]
enum Message {
    InitializeChatError(String),
    MessagePost(Vec<UserMessage>),
    // If bool is true, then the stream has been initialized in the
    // subscription successfully.
    MessageStreamInitState(bool),
    Terminate,
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "Message <>")
    }
}

struct StreamChat {
    // If true, the message stream has already spawned a subscription
    // that will feed messages to the ui.
    subscribed_to_message_stream: bool,
    // stream: Option<Arc<dyn MessageStream>>,
    active_messages: VecDeque<UserMessage>,
}

impl StreamChat {
    fn boot() -> (Self, iced::Task<Message>) {
        (StreamChat::new(), iced::Task::none())
    }

    fn new() -> Self {
        StreamChat {
            subscribed_to_message_stream: false,
            active_messages: VecDeque::new(),
        }
    }

    fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::InitializeChatError(error_message) => {
                // TODO: display the error in some modal.
                eprintln!("Erorr: {}", error_message);
                iced::Task::done(Message::Terminate)
            }
            Message::MessagePost(messages) => {
                for message in messages {
                    self.active_messages.push_back(message);
                }
                // TODO: Optimize, do not remove one by one...
                while self.active_messages.len() > 100 {
                    self.active_messages.pop_front();
                }
                iced::Task::none()
            }
            Message::MessageStreamInitState(stream_init_success) => {
                self.subscribed_to_message_stream = stream_init_success;
                iced::Task::none()
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

    fn subscription(&self) -> iced::Subscription<Message> {
        iced::Subscription::run(message_stream_sub)
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
                // Filler to supress compiler.
                .on_link_click(|_link: u32| Message::Terminate)
                .size(14),
            );
        }
        v.into()
    }
}

async fn create_message_stream_initializer(
    url: &str,
) -> std::result::Result<Box<dyn MessageStream>, String> {
    let url_parts = url.splitn(2, "://").collect::<Vec<_>>();
    if url_parts.len() != 2 {
        return Err(String::from("Invalid url, no protocol found."));
    }

    let (protocol, path) = (url_parts[0], url_parts[1]);
    match protocol {
        "file" => Ok(Box::new(CsvMessageStream::new_from_file(path))),
        "twitch" => Ok(Box::new(TwitchMessageStream::new_for_stream(path).await)),
        _ => Err(String::from(format!("Invalid protocol: {}", protocol))),
    }
}

// Subscription that reads messages from the chat websocket continuously
// and outputs the received messages back to the application's update.
fn message_stream_sub() -> impl iced::task::Sipper<iced::task::Never, Message> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        panic!("Expected argument to chat stream.");
    }
    iced::task::sipper(async |mut output| {
        loop {
            let args: Vec<String> = std::env::args().collect();
            if args.len() < 2 {
                panic!("Expected argument to chat stream.");
            }
            let url = args[1].clone();

            let stream = create_message_stream_initializer(url.as_str()).await;
            if let Err(e) = stream {
                output.send(Message::InitializeChatError(e)).await;
            } else if let Ok(mut stream) = stream {
                output.send(Message::MessageStreamInitState(true)).await;
                loop {
                    let user_messages = stream.collect_messages().await;
                    if user_messages.len() > 0 {
                        println!("Received {} messages, sending...", user_messages.len());
                        output.send(Message::MessagePost(user_messages)).await;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
    })
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
trait MessageStream: AToAny + Send + Sync {
    async fn collect_messages(&mut self) -> Vec<UserMessage>;
}

struct CsvMessageStream {
    messages: VecDeque<UserMessage>,
}

impl Clone for CsvMessageStream {
    fn clone(&self) -> Self {
        CsvMessageStream {
            messages: self.messages.clone(),
        }
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

#[async_trait]
impl MessageStream for CsvMessageStream {
    async fn collect_messages(&mut self) -> Vec<UserMessage> {
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
        if nb_released > 0 {
            println!(
                "CsvMessageStream: nb_released={} nb_remaining={}",
                nb_released,
                self.messages.len()
            );
        }

        released_messages
    }
}

struct TwitchMessageStream {
    stream_name: String,
    message_stream_received: Option<tokio::sync::mpsc::Receiver<UserMessage>>,
}

impl std::fmt::Debug for TwitchMessageStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "TwitchMessageStream <channel = {}>", self.stream_name)
    }
}

impl Clone for TwitchMessageStream {
    fn clone(&self) -> Self {
        if self.message_stream_received.is_some() {
            eprintln!("TwitchMessageStream cloned, message_stream_receiver not preserved.");
        }
        TwitchMessageStream {
            stream_name: self.stream_name.clone(),
            message_stream_received: None,
        }
    }
}

impl TwitchMessageStream {
    async fn new_for_stream(stream: &str) -> Self {
        let stream_rx = match setup_twitch_oauth(stream).await {
            Err(twitch_auth_err) => {
                eprintln!("error: {}", twitch_auth_err);
                None
            }
            Ok(message_stream_receiver) => message_stream_receiver,
        };
        if stream_rx.is_none() {
            eprintln!("No stream receiver returned during initialization...");
        }
        TwitchMessageStream {
            stream_name: String::from(stream),
            message_stream_received: stream_rx,
        }
    }
}

#[async_trait]
impl MessageStream for TwitchMessageStream {
    async fn collect_messages(&mut self) -> Vec<UserMessage> {
        match self.message_stream_received.as_mut() {
            None => Vec::new(),
            Some(rx) => {
                // TODO: Instead of creating a vector for each message, either get rid of the
                // vector or batch the messages together.
                if let Some(message) = rx.recv().await {
                    vec![message]
                } else {
                    Vec::new()
                }
            }
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
async fn setup_twitch_oauth(
    stream_name: &str,
) -> Result<Option<tokio::sync::mpsc::Receiver<UserMessage>>, reqwest::Error> {
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
        return Ok(None);
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
        stream_name,
        api_config.twitch_api_client_id.as_str(),
        app_config.twitch_auth.unwrap().access_token.as_str(),
    )
    .await
    {
        Ok(Some(rx)) => {
            println!("auth_twitch_chat_event_sub: successful");
            Ok(Some(rx))
        }
        Ok(None) => {
            println!("auth_twitch_chat_event_sub: failed..");
            Ok(None)
        }
        Err(err) => {
            eprintln!("auth_twitch_chat_event_sub: err={:?}", err);
            Ok(None)
        }
    }
}

async fn auth_twitch_chat_event_sub(
    stream_name: &str,
    client_id: &str,
    access_token: &str,
) -> Result<Option<tokio::sync::mpsc::Receiver<UserMessage>>, reqwest::Error> {
    let twitch_ws_url = "wss://eventsub.wss.twitch.tv/ws";

    let (ws_stream, _) = tokio_tungstenite::connect_async(twitch_ws_url)
        .await
        .expect("Failed to connect to Twitch EventSub websocket.");
    let (_, read) = ws_stream.split();
    println!(
        "Reading messages from the websocket endpoint: ({})...",
        twitch_ws_url
    );

    let mut session_id: Option<String> = None;
    let mut ws_enumerate = read.enumerate();
    while let next_message_result = ws_enumerate.next().await {
        match next_message_result {
            Some((_, Ok(message))) => {
                let data = message.into_text().unwrap();
                println!("ws message: {}", data);

                if let Ok(payload) = serde_json::from_str::<serde_json::Value>(data.as_str()) {
                    // Try to parse some session id.
                    let parsed_session = payload
                        .get("payload")
                        .and_then(|value| value.get("session"));
                    if let Some(status) = parsed_session
                        .and_then(|value| value.get("status"))
                        .and_then(|value| value.as_str())
                    {
                        println!("Found websocket status from payload.");
                        if status == "connected" {
                            println!("websocket status=connected");
                            let parsed_session_id = parsed_session
                                .and_then(|value| value.get("id"))
                                .and_then(|value| value.as_str())
                                .unwrap();
                            println!("websocket session id={}", parsed_session_id);
                            session_id = Some(String::from(parsed_session_id));
                            break;
                        } else {
                            eprintln!("Invalid websocket status: {}", status);
                        }
                    } else {
                        println!("Could not find websocket status from payload.");
                    }
                }
            }
            _ => {
                eprintln!("Error: failed to get message from websocket stream...");
            }
        }
    }
    assert!(session_id.is_some());

    println!("Attempting to subscribe to chat event sub using this websocket session...");
    match auth_twitch_chat_event_sub_init(
        stream_name,
        session_id.unwrap().as_str(),
        client_id,
        access_token,
    )
    .await
    {
        Ok(success) => {
            println!("Subscribed to eventsub={}", success);
            if !success {
                return Ok(None);
            }
        }
        Err(err) => {
            eprintln!("Failed to subscribe to eventsub: err={:?}", err);
            return Ok(None);
        }
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<UserMessage>(100);

    // TODO: Use cancelation thread to control the read stream...
    println!("Spawning task to read messages from stream...");
    tokio::spawn(async move {
        let (log_chat_messages, log_payload_message) = (false, false);
        loop {
            match ws_enumerate.next().await {
                Some((_, Ok(message))) => {
                    if log_payload_message {
                        println!("payload={}", message);
                    }

                    // Parse the message and chatter name.
                    if let Ok(json_data) = serde_json::from_str::<serde_json::Value>(
                        message.into_text().unwrap().as_str(),
                    ) {
                        let event_data = json_data
                            .get("payload")
                            .and_then(|value| value.get("event"));
                        let chatter_username = event_data
                            .and_then(|value| value.get("chatter_user_name"))
                            .and_then(|value| value.as_str());
                        // TODO: There is also a message.fragments field that
                        // partitions the message into its message, emote and mention components.
                        let chatter_message = event_data
                            .and_then(|value| value.get("message"))
                            .and_then(|value| value.get("text"))
                            .and_then(|value| value.as_str());

                        if let (Some(chatter_username), Some(chatter_message)) =
                            (chatter_username, chatter_message)
                        {
                            if let Err(_) = tx
                                .send(UserMessage {
                                    username: String::from(chatter_username),
                                    message: String::from(chatter_message),
                                    timestamp: std::time::Instant::now(), // TODO: parse the timestamp from the payload...
                                })
                                .await
                            {
                                eprintln!("Failed to send chatter message to mpsc.");
                            }
                            if log_chat_messages {
                                println!("-> {}: {}", chatter_username, chatter_message);
                            }
                        }
                    }
                }
                _ => {
                    eprintln!("Could not get next message from websocket... exiting read loop.");
                    break;
                }
            }
        }
        println!("Exiting weboscket task.");
    });
    Ok(Some(rx))
}

async fn twitch_get_user_id_from_name(
    user_name: &str,
    client_id: &str,
    access_token: &str,
) -> Result<Option<String>, reqwest::Error> {
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

    let response = client
        .get(format!(
            "https://api.twitch.tv/helix/users?login={}",
            user_name
        ))
        .headers(headers)
        .send()
        .await?;

    if response.status() != reqwest::StatusCode::OK {
        Ok(None)
    } else {
        #[derive(Deserialize)]
        struct InlineTwitchUserIdPayload {
            id: String,
        }
        #[derive(Deserialize)]
        struct InlineTwitchUserIdDataPayload {
            data: Vec<InlineTwitchUserIdPayload>,
        }

        let data = response.text().await.unwrap();
        println!("user_id_data={}", data);
        let res: InlineTwitchUserIdDataPayload =
            serde_json::from_str(data.as_str()).expect("Failed to parse twitch user id payload.");

        if res.data.len() == 0 {
            eprintln!("No user id found in payload...");
            Ok(None)
        } else {
            Ok(Some(res.data[0].id.clone()))
        }
    }
}

// Subscribe to the twitch event sub using the provided `websocket_session_id`.
async fn auth_twitch_chat_event_sub_init(
    stream_name: &str,
    websocket_session_id: &str,
    client_id: &str,
    access_token: &str,
) -> Result<bool, reqwest::Error> {
    let streamer_id = twitch_get_user_id_from_name(stream_name, client_id, access_token).await?;
    if let None = streamer_id {
        eprintln!("Invalid twitch user: {}", stream_name);
        return Ok(false);
    }
    let streamer_id = streamer_id.unwrap();

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
            broadcaster_user_id: String::from(streamer_id),
            // TODO: Do not hardcode the user IDs into the request...
            user_id: String::from("76000742" /* infallible_mob_ */),
        },
        transport: InlineTransport {
            method: String::from("websocket"),
            session_id: String::from(websocket_session_id),
        },
    };

    let response = client
        .post("https://api.twitch.tv/helix/eventsub/subscriptions")
        .headers(headers)
        .json(&body)
        .send()
        .await?;

    println!("EventSub.channel_messages = Response={}", response.status());
    if response.status() != reqwest::StatusCode::OK
        && response.status() != reqwest::StatusCode::ACCEPTED
    {
        eprintln!("Bad response status...");
        if let Ok(response_body) = response.text().await {
            eprintln!("Response body={}", response_body);
        }
        return Ok(false);
    }

    let response_msg = response.text().await.unwrap();
    println!("EventSub response={}", response_msg);
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
