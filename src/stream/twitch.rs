use crate::stream::base::UserMessage;
use crate::stream::base::MessageStream;
use crate::app_util::{AppData, ApiConfig, AppConfig};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use iced::futures::StreamExt;

pub struct TwitchMessageStream {
    stream_name: String,
    message_stream_received: Option<tokio::sync::mpsc::Receiver<UserMessage>>,
    // Token used to cancel the stream that processes the messages from the
    // stream connection.
    cancel_task: Option<tokio_util::sync::CancellationToken>,
    user_id: String,
    broadcaster_id: String,
}

impl std::fmt::Debug for TwitchMessageStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "TwitchMessageStream <channel = {}>", self.stream_name)
    }
}

impl TwitchMessageStream {
    pub async fn new_for_stream(stream: &str) -> Self {
        let (api_config, app_config) = AppData::get_configs().await;
        let (broadcaster_id, user_id) =
            Self::fetch_broadcaster_and_user_id(&api_config, &app_config, stream)
                .await
                .unwrap();

        println!("Twitch:Broadcaster ID: {}", broadcaster_id);
        println!("Twitch:User ID: {}", user_id);
        let stream_conn = setup_twitch_oauth(
            &api_config,
            &app_config,
            broadcaster_id.as_str(),
            user_id.as_str(),
        )
        .await;
        if stream_conn.is_err() {
            let err = stream_conn.err().unwrap();
            eprintln!(
                "Failed to receive stream receiver or cancellation token..: {}",
                err
            );
            TwitchMessageStream {
                stream_name: String::from(stream),
                message_stream_received: None,
                cancel_task: None,
                user_id: String::new(),
                broadcaster_id: String::new(),
            }
        } else {
            let (stream_rx, cancel_task) = stream_conn.unwrap().unwrap();
            TwitchMessageStream {
                stream_name: String::from(stream),
                message_stream_received: Some(stream_rx),
                cancel_task: Some(cancel_task),
                user_id,
                broadcaster_id,
            }
        }
    }

    async fn fetch_broadcaster_and_user_id(
        api_config: &ApiConfig,
        app_config: &AppConfig,
        stream_name: &str,
    ) -> Option<(String, String)> {
        let access_token = app_config
            .twitch_auth
            .as_ref()
            .unwrap()
            .access_token
            .as_str();
        let client_id = api_config.twitch_api_client_id.as_str();

        let streamer_id = get_user_id_from_name(Some(stream_name), client_id, access_token)
            .await
            .expect("Failed to get streamer id...");
        if let None = streamer_id {
            eprintln!("Invalid twitch user: {}", stream_name);
            return None;
        }
        let streamer_id = streamer_id.unwrap();
        let user_id = get_user_id_from_name(None, client_id, access_token)
            .await
            .expect("Failed to get user id.");
        if let None = user_id {
            eprintln!("Failed to get user id for current authenticated user.");
            return None;
        }
        let user_id = user_id.unwrap();

        Some((streamer_id, user_id))
    }
}

#[async_trait]
impl MessageStream for TwitchMessageStream {
    fn get_broadcaster_and_user_id(
        &self,
    ) -> Option<(
        String,
        String,
        String,
        Option<tokio_util::sync::CancellationToken>,
    )> {
        Some((
            self.broadcaster_id.clone(),
            self.stream_name.clone(),
            self.user_id.clone(),
            self.cancel_task.clone(),
        ))
    }

    async fn next_message(&mut self) -> Option<UserMessage> {
        match self.message_stream_received.as_mut() {
            None => None,
            Some(rx) => rx.recv().await,
        }
    }
}

// If user_name is None, it will get the user id for the current oauth token.
// Otherwise, it wil lget the user id for the requested twitch user_name.
async fn get_user_id_from_name(
    user_name: Option<&str>,
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

    let url = match user_name {
        Some(user_name) => format!("https://api.twitch.tv/helix/users?login={}", user_name),
        None => String::from("https://api.twitch.tv/helix/users"),
    };
    let response = client.get(url).headers(headers).send().await?;

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

// Setup the twitch user authentication.
async fn setup_twitch_oauth(
    api_config: &ApiConfig,
    app_config: &AppConfig,
    broadcaster_id: &str,
    user_id: &str,
) -> Result<
    Option<(
        tokio::sync::mpsc::Receiver<UserMessage>,
        tokio_util::sync::CancellationToken,
    )>,
    reqwest::Error,
> {
    // Subscribe to the EventSub for receiving chat messages.
    println!("Attempting to subscribe to twitch chat event sub.");
    match auth_twitch_chat_event_sub(
        broadcaster_id,
        user_id,
        api_config.twitch_api_client_id.as_str(),
        app_config
            .twitch_auth
            .as_ref()
            .unwrap()
            .access_token
            .as_str(),
    )
    .await
    {
        Ok(Some(result)) => {
            println!("auth_twitch_chat_event_sub: successful");
            Ok(Some(result))
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
    broadcaster_id: &str,
    user_id: &str,
    client_id: &str,
    access_token: &str,
) -> Result<
    Option<(
        tokio::sync::mpsc::Receiver<UserMessage>,
        tokio_util::sync::CancellationToken,
    )>,
    reqwest::Error,
> {
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
        broadcaster_id,
        user_id,
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

    let token = tokio_util::sync::CancellationToken::new();
    let cloned_token = token.clone();
    println!("Spawning task to read messages from stream...");
    tokio::spawn(async move {
        tokio::select! {
          _ = cloned_token.cancelled() => {
          }
          _ = process_websocket_event(tx, ws_enumerate) => {}
        }
    });
    Ok(Some((rx, token)))
}

async fn process_websocket_event(
    mpsc_sender: tokio::sync::mpsc::Sender<UserMessage>,
    mut websocket_enumerate: futures_util::stream::Enumerate<
        futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
    >,
) {
    let (log_chat_messages, log_payload_message) = (false, false);
    loop {
        // println!("Twitch EventSubLoop: Waiting for next message...");
        match websocket_enumerate.next().await {
            Some((_, Ok(message))) => {
                if log_payload_message {
                    println!("payload={}", message);
                }

                // Parse the message and chatter name.
                if let Ok(json_data) =
                    serde_json::from_str::<serde_json::Value>(message.into_text().unwrap().as_str())
                {
                    let event_data = json_data
                        .get("payload")
                        .and_then(|value| value.get("event"));
                    let chatter_username = event_data
                        .and_then(|value| value.get("chatter_user_name"))
                        .and_then(|value| value.as_str());
                    let chatter_user_id = event_data
                        .and_then(|value| value.get("chatter_user_id"))
                        .and_then(|value| value.as_str());
                    let broadcaster_user_id = event_data
                        .and_then(|value| value.get("broadcaster_user_id"))
                        .and_then(|value| value.as_str());
                    let broadcaster_user_name = event_data
                        .and_then(|value| value.get("broadcaster_user_name"))
                        .and_then(|value| value.as_str());
                    // TODO: There is also a message.fragments field that
                    // partitions the message into its message, emote and mention components.
                    let chatter_message = event_data
                        .and_then(|value| value.get("message"))
                        .and_then(|value| value.get("text"))
                        .and_then(|value| value.as_str());

                    if let (
                        Some(chatter_username),
                        Some(chatter_user_id),
                        Some(chatter_message),
                        Some(broadcaster_user_id),
                        Some(broadcaster_user_name),
                    ) = (
                        chatter_username,
                        chatter_user_id,
                        chatter_message,
                        broadcaster_user_id,
                        broadcaster_user_name,
                    ) {
                        let broadcaster_user_name = broadcaster_user_name.to_lowercase();
                        if let Err(_) = mpsc_sender
                            .send(UserMessage {
                                user_id: String::from(chatter_user_id),
                                username: String::from(chatter_username),
                                message: String::from(chatter_message),
                                broadcast_id: String::from(broadcaster_user_id),
                                broadcaster_name: broadcaster_user_name,
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
}

// Subscribe to the twitch event sub using the provided `websocket_session_id`.
async fn auth_twitch_chat_event_sub_init(
    broadcaster_id: &str,
    user_id: &str,
    websocket_session_id: &str,
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
            broadcaster_user_id: String::from(broadcaster_id),
            user_id: String::from(user_id),
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

pub async fn send_message(
    broadcaster_id: String,
    user_id: String,
    api_config: ApiConfig,
    app_config: AppConfig,
    message: String,
) {
    let access_token = app_config.twitch_auth.unwrap().access_token;
    let client_id = api_config.twitch_api_client_id;

    let client = reqwest::Client::new();
    let bearer_str = format!("Bearer {}", access_token);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "Authorization",
        reqwest::header::HeaderValue::from_str(bearer_str.as_str()).unwrap(),
    );
    headers.insert(
        "Client-Id",
        reqwest::header::HeaderValue::from_str(client_id.as_str()).unwrap(),
    );
    headers.insert(
        "Content-Type",
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    #[derive(Serialize)]
    struct TwitchMessageSendBody {
        broadcaster_id: String,
        sender_id: String,
        message: String,
    }

    let body = TwitchMessageSendBody {
        broadcaster_id,
        sender_id: user_id,
        message,
    };

    let res = client
        .post("https://api.twitch.tv/helix/chat/messages")
        .headers(headers)
        .json(&body)
        .send()
        .await
        .unwrap();
    println!("Message send status={}", res.status());
    ()
}
