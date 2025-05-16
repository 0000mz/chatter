use async_trait::async_trait;
use iced::futures::StreamExt;
use iced::widget::{column, rich_text, row, span};
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
        .title(StreamChat::title)
        .style(StreamChat::style)
        .run()
}

#[derive(Clone)]
enum Message {
    InitializeApp,
    UpdateAppConfig(Option<(ApiConfig, AppConfig)>),
    InitializeChatError(String),
    MessagePost(UserMessage),
    // If bool is true, then the stream has been initialized in the
    // subscription successfully.
    MessageStreamInitState(bool),
    InputMessageChanged(String),
    SaveBroadcasterInfo((String, String)),
    SaveUserId(String),
    SendInputMessage,
    // If true, the command palette will be set to active.
    // Otherwise, it will be set to inactive.
    ToggleCommandPalette(bool),
    CommandPaletteSearchChanged(String),
    // Selects the currently selected command palette option.
    CommandPaletteSelect,
    // Received when the ESC key is pressed. What should happen depends
    // entirely on the current state of the application.
    // i.e. if the current focus is the command palette, then exit it.
    HandleSpecialKey(iced::keyboard::key::Named),
    // If this message is received, the focus should turn to the
    // current open chat area.
    FocusChatArea,
    Nothing(()),
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
    active_messages: VecDeque<UserMessage>,
    input_message: String,
    api_config: Option<ApiConfig>,
    app_config: Option<AppConfig>,
    broadcaster_id: Option<String>,
    broadcaster_name: Option<String>,
    user_id: Option<String>,
    command_palette_ctx: CommandPalette,
}

impl StreamChat {
    fn boot() -> (Self, iced::Task<Message>) {
        (StreamChat::new(), iced::Task::done(Message::InitializeApp))
    }

    fn new() -> Self {
        StreamChat {
            subscribed_to_message_stream: false,
            active_messages: VecDeque::new(),
            input_message: String::new(),
            api_config: None,
            app_config: None,
            broadcaster_id: None,
            broadcaster_name: None,
            user_id: None,
            command_palette_ctx: CommandPalette::new(),
        }
    }

    fn style(&self, _: &iced::Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: iced::color!(0x000000),
            text_color: iced::color!(0xffffff),
        }
    }

    fn title(&self) -> String {
        if let Some(broadcaster_name) = self.broadcaster_name.as_ref() {
            format!("StreamChat - [{}]", broadcaster_name)
        } else {
            String::from("StreamChat - [unnamed]")
        }
    }

    fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::InitializeApp => iced::Task::batch([
                iced::Task::perform(
                    (|| async move {
                        let res = AppData::setup().await.unwrap();
                        res
                    })(),
                    Message::UpdateAppConfig,
                ),
                iced::Task::done(Message::FocusChatArea),
            ]),
            Message::UpdateAppConfig(config) => match config {
                None => iced::Task::done(Message::InitializeChatError(String::from(
                    "Failed to initialize config.",
                ))),
                Some((api_config, app_config)) => {
                    self.api_config = Some(api_config);
                    self.app_config = Some(app_config);
                    iced::Task::none()
                }
            },
            Message::InitializeChatError(error_message) => {
                // TODO: display the error in some modal.
                eprintln!("Erorr: {}", error_message);
                iced::Task::done(Message::Terminate)
            }
            Message::MessagePost(message) => {
                self.active_messages.push_back(message);
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
            Message::InputMessageChanged(message) => {
                self.input_message = message;
                iced::Task::none()
            }
            Message::CommandPaletteSearchChanged(search) => {
                self.command_palette_ctx.update_current_from_query(search);
                iced::Task::none()
            }
            Message::CommandPaletteSelect => {
                self.command_palette_ctx.maybe_select();
                iced::Task::none()
            }
            Message::SendInputMessage => {
                let message = self.input_message.trim();
                if message.len() == 0 {
                    iced::Task::none()
                } else {
                    let message = String::from(message);
                    self.input_message = String::new();
                    // TODO: Probably don't need to clone this for every message sent.
                    let api_config = self.api_config.as_ref().unwrap().clone();
                    let app_config = self.app_config.as_ref().unwrap().clone();

                    if let (Some(broadcaster_id), Some(user_id)) =
                        (self.broadcaster_id.as_ref(), self.user_id.as_ref())
                    {
                        let broadcaster_id = broadcaster_id.clone();
                        let user_id = user_id.clone();
                        iced::Task::perform(
                            (|| async move {
                                twitch_send_message(
                                    broadcaster_id.clone(),
                                    user_id.clone(),
                                    api_config,
                                    app_config,
                                    message,
                                )
                                .await;
                            })(),
                            Message::Nothing,
                        )
                    } else {
                        eprintln!("Not sending message.. no broadcaster/user id found in state...");
                        iced::Task::none()
                    }
                }
            }
            Message::SaveUserId(user_id) => {
                self.user_id = Some(user_id);
                iced::Task::none()
            }
            Message::SaveBroadcasterInfo((broadcaster_id, broadcaster_name)) => {
                self.broadcaster_id = Some(broadcaster_id);
                self.broadcaster_name = Some(broadcaster_name);
                iced::Task::none()
            }
            Message::ToggleCommandPalette(enabled) => {
                self.command_palette_ctx.active = enabled;
                iced::widget::text_input::focus("command-palette-input")
            }
            Message::FocusChatArea => iced::widget::text_input::focus("chat-message-input"),
            Message::HandleSpecialKey(key) => match key {
                iced::keyboard::key::Named::Escape => {
                    if self.command_palette_ctx.active {
                        self.command_palette_ctx.active = false;
                        self.command_palette_ctx
                            .update_current_from_query(String::new());
                        iced::Task::done(Message::FocusChatArea)
                    } else {
                        iced::Task::none()
                    }
                }
                arrow @ iced::keyboard::key::Named::ArrowUp
                | arrow @ iced::keyboard::key::Named::ArrowDown => {
                    let delta = if arrow == iced::keyboard::key::Named::ArrowUp {
                        -1
                    } else {
                        1
                    };
                    if self.command_palette_ctx.active {
                        let mut next_index = self.command_palette_ctx.selected_index + delta;
                        if next_index < -1 {
                            next_index = -1;
                        }
                        if next_index >= self.command_palette_ctx.current.len() as i32 {
                            next_index = self.command_palette_ctx.current.len() as i32 - 1;
                        }
                        self.command_palette_ctx.selected_index = next_index;
                    }
                    iced::Task::none()
                }
                _ => iced::Task::none(),
            },
            Message::Nothing(_) => iced::Task::none(),
        }
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        if let (Some(_), Some(_)) = (&self.api_config, &self.app_config) {
            let keypress_sub = iced::keyboard::on_key_release(|key, mods| {
                match (key.as_ref(), mods) {
                    (iced::keyboard::Key::Named(k @ iced::keyboard::key::Named::Escape), _)
                    | (iced::keyboard::Key::Named(k @ iced::keyboard::key::Named::ArrowUp), _)
                    | (iced::keyboard::Key::Named(k @ iced::keyboard::key::Named::ArrowDown), _) => {
                        Some(Message::HandleSpecialKey(k))
                    }
                    (iced::keyboard::Key::Character("p"), iced::keyboard::Modifiers::CTRL) => {
                        Some(Message::ToggleCommandPalette(true))
                    }
                    _ => None,
                }
            });
            let message_stream_sub = iced::Subscription::run(message_stream_sub);
            iced::Subscription::batch([keypress_sub, message_stream_sub])
        } else {
            iced::Subscription::none()
        }
    }

    fn view(&self) -> iced::Element<Message> {
        let mut v = column![];
        for msg in &self.active_messages {
            v = v.push(
                iced::widget::container(
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
                )
                .style(
                    if let Some(user_id) = self.user_id.as_ref()
                        && msg.user_id.as_str() == user_id.as_str()
                    {
                        AppStyle::highlighted_comment
                    } else {
                        AppStyle::unhighlighted_comment
                    },
                )
                .padding([0, 10])
                .width(iced::Fill),
            );
        }

        let base_ui = column![
            iced::widget::container(row![
                iced::widget::container(iced::widget::text(
                    if let Some(name) = self.broadcaster_name.as_ref() {
                        name
                    } else {
                        "unnamed"
                    }
                ))
                .style(|_theme| {
                    // TODO: reused style -- Add this to some global theme.
                    let outline_color = color!(0x4828ad);
                    iced::widget::container::background(outline_color)
                })
                .padding([5, 10])
            ])
            .width(iced::Fill)
            .style(|_theme| {
                // TODO: reused style -- Add this to some global theme.
                let outline_color = color!(0x2d1870);
                iced::widget::container::background(outline_color)
            }),
            iced::widget::scrollable(v)
                .width(iced::Fill)
                .height(iced::Fill)
                .anchor_bottom(),
            row![
                iced::widget::text_input("Send chat message...", self.input_message.as_str())
                    .id("chat-message-input")
                    .on_input(Message::InputMessageChanged)
                    .on_submit(Message::SendInputMessage)
                    .width(iced::Fill)
                    .padding([10, 20])
                    .style(|theme, status| {
                        // TODO: reused style -- Add this to some global theme.
                        let outline_color = color!(0x4828ad);
                        let mut style = iced::widget::text_input::default(theme, status);
                        style.border.radius = iced::border::left(0.0);
                        style.border.color = outline_color;
                        style.background = iced::Background::Color(color!(0x000000));
                        style
                    }),
                iced::widget::button("Send")
                    .on_press_maybe((|| {
                        if self.input_message.len() > 0 {
                            Some(Message::SendInputMessage)
                        } else {
                            None
                        }
                    })())
                    .style(|theme, status| {
                        // TODO: reused style -- Add this to some global theme.
                        let outline_color = color!(0x4828ad);
                        let mut style = iced::widget::button::primary(theme, status);
                        style.border.radius = iced::border::left(0.0);
                        style.background = Some(iced::Background::Color(outline_color));
                        style
                    })
                    .padding([10, 20])
            ]
        ];

        // TODO: make the command palette width not larger than the window's width.
        let command_palette_width = 600;
        let command_palette_layer = column![
            iced::widget::vertical_space().height(200),
            row![
                iced::widget::horizontal_space(),
                iced::widget::text_input(
                    "Command Palette",
                    self.command_palette_ctx.query.as_str()
                )
                .on_input(Message::CommandPaletteSearchChanged)
                .on_submit(Message::CommandPaletteSelect)
                .id("command-palette-input")
                .width(command_palette_width)
                .padding([10, 10])
                .size(14)
                .style(if self.command_palette_ctx.current.is_empty() {
                    AppStyle::command_palette_text_input_no_results
                } else {
                    AppStyle::command_palette_text_input_with_results
                }),
                iced::widget::horizontal_space(),
            ],
            row![
                iced::widget::horizontal_space(),
                iced::widget::container(self.command_palette_results_view())
                    .width(command_palette_width)
                    .style(|_theme| { iced::widget::container::background(color!(0xffffff)) }),
                iced::widget::horizontal_space(),
            ],
        ];
        iced::widget::stack![base_ui]
            .push_maybe(if self.command_palette_ctx.active {
                Some(command_palette_layer)
            } else {
                None
            })
            .into()
    }

    fn command_palette_results_view(&self) -> iced::Element<Message> {
        let mut results = column![];
        // TODO: visibility (1) -- bold/highlight the parts of the option that match the query.
        // TODO: visibility (2) -- highlight the background of the current selected option.
        for (i, option) in self.command_palette_ctx.current.iter().enumerate() {
            let mut entry = iced::widget::container(
                iced::widget::text(option).size(14).color(color!(0x3d691f)),
            )
            .width(iced::Fill)
            .padding([5, 10]);
            if self.command_palette_ctx.selected_index == i as i32 {
                entry = entry.style(|_theme| iced::widget::container::background(color!(0xcf7c76)));
            }
            results = results.push(entry);
        }
        results.into()
    }
}

struct AppStyle;
impl AppStyle {
    fn highlighted_comment(_theme: &iced::widget::Theme) -> iced::widget::container::Style {
        iced::widget::container::background(color!(0xad3939, 0.2))
    }

    fn unhighlighted_comment(theme: &iced::widget::Theme) -> iced::widget::container::Style {
        iced::widget::container::transparent(theme)
    }

    fn command_palette_text_input_with_results(
        theme: &iced::widget::Theme,
        status: iced::widget::text_input::Status,
    ) -> iced::widget::text_input::Style {
        let mut style = iced::widget::text_input::default(theme, status);
        style.border.radius.top_right = 5.0;
        style.border.radius.top_left = 5.0;
        style.border.radius.bottom_right = 0.0;
        style.border.radius.bottom_left = 0.0;
        style
    }

    fn command_palette_text_input_no_results(
        theme: &iced::widget::Theme,
        status: iced::widget::text_input::Status,
    ) -> iced::widget::text_input::Style {
        let mut style = iced::widget::text_input::default(theme, status);
        style.border.radius.top_right = 5.0;
        style.border.radius.top_left = 5.0;
        style.border.radius.bottom_right = 5.0;
        style.border.radius.bottom_left = 5.0;
        style
    }
}

struct CommandPalette {
    actions: std::collections::HashMap<String, String>,
    // The current relevant actions based on the user's query.
    current: Vec<String>,
    query: String,
    // If true, the command palette is in focus and being used.
    active: bool,
    // The option in the commant palette list that is highlighted.
    selected_index: i32,
}

impl CommandPalette {
    fn new() -> Self {
        Self {
            actions: std::collections::HashMap::from([
                // Quit the application...
                (String::from("quit"), String::from("execute quit action")),
            ]),
            current: vec![],
            query: String::new(),
            active: false,
            selected_index: -1,
        }
    }

    // Try to select the current highlighted option.
    // If no option is highlighted, nothing is done.
    fn maybe_select(&self) -> Option<()> {
        if self.selected_index >= self.current.len() as i32 || self.selected_index < 0 {
            None
        } else {
            let selected_key = &self.current[self.selected_index as usize];
            let action_exec = self.actions.get(selected_key).expect("Unknown action");
            println!("TODO: do something with the action: => \"{}\"", action_exec);
            None
        }
    }

    fn update_current_from_query(&mut self, query: String) {
        let mut current_actions = vec![];
        let query = query.trim();
        if !query.is_empty() {
            for (k, _) in &self.actions {
                if k.starts_with(query) {
                    current_actions.push(k.clone());
                }
            }
        }
        self.current = current_actions;
        self.query = String::from(query);
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
                if let Some((broadcaster_id, broadcaster_name, user_id)) =
                    stream.get_broadcaster_and_user_id()
                {
                    output
                        .send(Message::SaveBroadcasterInfo((
                            broadcaster_id,
                            broadcaster_name,
                        )))
                        .await;
                    output.send(Message::SaveUserId(user_id)).await;
                }

                output.send(Message::MessageStreamInitState(true)).await;
                loop {
                    let user_message = stream.next_message().await;
                    if let Some(user_message) = user_message {
                        output.send(Message::MessagePost(user_message)).await;
                    }
                    tokio::time::sleep(std::time::Duration::from_nanos(100)).await;
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
    async fn next_message(&mut self) -> Option<UserMessage>;
    fn get_broadcaster_and_user_id(&self) -> Option<(String, String, String)>;
}

struct CsvMessageStream {
    messages: VecDeque<UserMessage>,
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
                parts[0], // username as user id
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
    fn get_broadcaster_and_user_id(&self) -> Option<(String, String, String)> {
        None
    }

    async fn next_message(&mut self) -> Option<UserMessage> {
        let mut itr = self.messages.iter();

        let mut next_user_message = None;
        if let Some(message) = itr.next()
            && std::time::Instant::now() >= message.timestamp
        {
            next_user_message = Some(message.clone());
        }
        if next_user_message.is_some() {
            self.messages.pop_front();
        }
        next_user_message
    }
}

struct TwitchMessageStream {
    stream_name: String,
    message_stream_received: Option<tokio::sync::mpsc::Receiver<UserMessage>>,
    user_id: String,
    broadcaster_id: String,
}

impl std::fmt::Debug for TwitchMessageStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "TwitchMessageStream <channel = {}>", self.stream_name)
    }
}

impl TwitchMessageStream {
    async fn new_for_stream(stream: &str) -> Self {
        let (api_config, app_config) = AppData::get_configs().await;
        let (broadcaster_id, user_id) =
            Self::fetch_broadcaster_and_user_id(&api_config, &app_config, stream)
                .await
                .unwrap();

        println!("Twitch:Broadcaster ID: {}", broadcaster_id);
        println!("Twitch:User ID: {}", user_id);
        let stream_rx = match setup_twitch_oauth(
            &api_config,
            &app_config,
            broadcaster_id.as_str(),
            user_id.as_str(),
        )
        .await
        {
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
            user_id,
            broadcaster_id,
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

        let streamer_id = twitch_get_user_id_from_name(Some(stream_name), client_id, access_token)
            .await
            .expect("Failed to get streamer id...");
        if let None = streamer_id {
            eprintln!("Invalid twitch user: {}", stream_name);
            return None;
        }
        let streamer_id = streamer_id.unwrap();
        let user_id = twitch_get_user_id_from_name(None, client_id, access_token)
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
    fn get_broadcaster_and_user_id(&self) -> Option<(String, String, String)> {
        Some((
            self.broadcaster_id.clone(),
            self.stream_name.clone(),
            self.user_id.clone(),
        ))
    }

    async fn next_message(&mut self) -> Option<UserMessage> {
        match self.message_stream_received.as_mut() {
            None => None,
            Some(rx) => rx.recv().await,
        }
    }
}

#[derive(Clone)]
struct UserMessage {
    user_id: String,
    username: String,
    message: String,
    timestamp: std::time::Instant,
}

impl UserMessage {
    fn new(user_id: &str, username: &str, message: &str, timestamp: std::time::Instant) -> Self {
        UserMessage {
            user_id: user_id.into(),
            username: username.into(),
            message: message.into(),
            timestamp,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct AppConfig {
    twitch_auth: Option<TwitchAuthPayload>,
}

#[derive(Deserialize, Clone)]
struct ApiConfig {
    twitch_api_client_id: String,
    twitch_api_secret: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct TwitchAuthPayload {
    access_token: String,
    refresh_token: String,
    #[serde(with = "serde_millis")]
    expiration_timestamp: std::time::Instant,
}

struct AppData;
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

    async fn get_configs() -> (ApiConfig, AppConfig) {
        let config_path = AppData::get_app_config_filepath();
        let config_contents = std::fs::read_to_string(&config_path).unwrap();
        let app_config: AppConfig = toml::from_str(config_contents.as_str()).unwrap();

        let api_config_path = AppData::get_api_config_filepath();
        let api_config_contents = std::fs::read_to_string(&api_config_path).unwrap();
        let api_config: ApiConfig = toml::from_str(api_config_contents.as_str()).unwrap();

        (api_config, app_config)
    }

    async fn setup() -> Result<Option<(ApiConfig, AppConfig)>, reqwest::Error> {
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

        Ok(Some(AppData::get_configs().await))
    }
}

// Setup the twitch user authentication.
async fn setup_twitch_oauth(
    api_config: &ApiConfig,
    app_config: &AppConfig,
    broadcaster_id: &str,
    user_id: &str,
) -> Result<Option<tokio::sync::mpsc::Receiver<UserMessage>>, reqwest::Error> {
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
    broadcaster_id: &str,
    user_id: &str,
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
                        let chatter_user_id = event_data
                            .and_then(|value| value.get("chatter_user_id"))
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
                        ) = (chatter_username, chatter_user_id, chatter_message)
                        {
                            if let Err(_) = tx
                                .send(UserMessage {
                                    user_id: String::from(chatter_user_id),
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

// If user_name is None, it will get the user id for the current oauth token.
// Otherwise, it wil lget the user id for the requested twitch user_name.
async fn twitch_get_user_id_from_name(
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

async fn twitch_send_message(
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
