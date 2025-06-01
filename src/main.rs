#![feature(vec_deque_truncate_front)]

mod stream;
use stream::base::EmoteDatabase;
use stream::base::MessageStream;
use stream::base::UserMessage;

mod app_util;
use app_util::{ApiConfig, AppConfig, AppData};

use iced::widget::{column, rich_text, row, span};
use iced::{Font, color, font};
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::vec::Vec;

const MAX_CHAT_MESSAGES: usize = 100;

fn main() -> iced::Result {
    iced::application(StreamChat::boot, StreamChat::update, StreamChat::view)
        .subscription(StreamChat::subscription)
        .title(StreamChat::title)
        .style(StreamChat::style)
        .run()
}

#[derive(Clone)]
enum ChatStreamMessage {
    InitializeChatError(String),
    // TODO: Use a struct instead of a tuple so that the fields are named.
    SaveBroadcasterInfo(String, String, tokio_util::sync::CancellationToken),
    SaveUserId(String),
    // If bool is true, then the stream has been initialized in the
    // subscription successfully.
    MessageStreamInitState(bool),
    MessagePost(UserMessage),
    InitializeEmoteDb,
}

#[derive(Clone)]
enum Message {
    InitializeApp,

    OpenChatStream(String),
    CloseActiveChatStream,
    ChatStreamError(Result<(), String>),
    UpdateAppConfig(Option<(ApiConfig, AppConfig)>),
    HandleChatStreamOutput(ChatStreamMessage),
    InputMessageChanged(String),
    SendInputMessage,
    SwitchActiveChat(String),
    // Attach the emote_db to the chat instance associated with the
    // stream_id (enum.0).
    AttachEmoteDatabase(String, crate::stream::base::EmoteDatabaseEnum),

    HandleError(String),
    // If true, the command palette will be set to active.
    // Otherwise, it will be set to inactive.
    // If true, the arg[1] will be set to the starting query string.
    CommandPaletteToggle(bool, Option<String>),
    CommandPaletteSearchChanged(String),
    // Selects the currently selected command palette option.
    CommandPaletteSelect,
    // Handle a command palette action where the first arg is the
    // action name and the second part are the arguments passed
    // to the action to be processed.
    CommandPaletteHandleAction(String, Option<Vec<String>>),
    // Selects the option at the given index of the command palette action
    // list.
    CommandPaletteHighlightActionIndex(i32),
    // Received when the ESC key is pressed. What should happen depends
    // entirely on the current state of the application.
    // i.e. if the current focus is the command palette, then exit it.
    HandleSpecialKey(iced::keyboard::key::Named),
    // If this message is received, the focus should turn to the
    // current open chat area.
    ChatViewportScroll(iced::widget::scrollable::AbsoluteOffset),
    FocusChatArea,
    Nothing(()),
    Terminate,
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "Message <>")
    }
}

struct ChatInstance {
    active_messages: VecDeque<UserMessage>,
    frozen_messages: VecDeque<UserMessage>,
    // Aborts the iced task that handles the read from the straw sipper.
    // Different from `cancel_task`, which controls the actual tokio async
    // operation.
    abort_stream: Option<iced::task::Handle>,
    // Cancellation token that will cancel the tokio stream used to read
    // the websocket stream.
    cancel_task: Option<tokio_util::sync::CancellationToken>,
    broadcaster_name: Option<String>,
    broadcaster_id: Option<String>,
    emote_db: Option<Box<dyn EmoteDatabase>>,
}

struct StreamChat {
    // If true, the message stream has already spawned a subscription
    // that will feed messages to the ui.
    subscribed_to_message_stream: bool,
    active_chat_instance: String,
    chat_instances: HashMap<String, ChatInstance>,
    input_message: String,
    api_config: Option<ApiConfig>,
    app_config: Option<AppConfig>,
    user_id: Option<String>,
    command_palette_ctx: CommandPalette,
    // If chat_frozen is true, new messages will not be rendered to the
    // chat output area.
    chat_frozen: bool,
}

impl StreamChat {
    fn boot() -> (Self, iced::Task<Message>) {
        (StreamChat::new(), iced::Task::done(Message::InitializeApp))
    }

    fn new() -> Self {
        StreamChat {
            subscribed_to_message_stream: false,
            active_chat_instance: String::new(),
            chat_instances: HashMap::new(),
            input_message: String::new(),
            api_config: None,
            app_config: None,
            user_id: None,
            command_palette_ctx: CommandPalette::new(),
            chat_frozen: false,
        }
    }

    fn style(&self, _: &iced::Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: iced::color!(0x000000),
            text_color: iced::color!(0xffffff),
        }
    }

    fn title(&self) -> String {
        if !self.active_chat_instance.is_empty() {
            format!("StreamChat - [{}]", self.active_chat_instance)
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
            Message::CloseActiveChatStream => {
                if self.chat_instances.contains_key(&self.active_chat_instance) {
                    let instance = self.chat_instances.get(&self.active_chat_instance).unwrap();
                    if let Some(cancel_task) = instance.cancel_task.as_ref() {
                        cancel_task.cancel();
                    } else {
                        eprintln!(
                            "CloseActiveChatStream: No cancellation token found for stream being closed..."
                        );
                    }
                    self.chat_instances.remove(&self.active_chat_instance);
                }

                // Find a different tab to set as active.
                let mut found_new_instance = false;
                if !self.chat_instances.is_empty() {
                    if let Some((stream_name, _)) = self.chat_instances.iter().next() {
                        self.active_chat_instance = stream_name.clone();
                        found_new_instance = true;
                    }
                }
                if !found_new_instance {
                    self.active_chat_instance = String::new();
                }
                iced::Task::none()
            }
            Message::OpenChatStream(stream_name) => {
                println!("Opening chat stream: {}", stream_name);

                if self.chat_instances.contains_key(&stream_name) {
                    eprintln!("Chat instance for stream already open: {}", stream_name);
                    iced::Task::none()
                } else {
                    let new_instance = ChatInstance {
                        active_messages: VecDeque::new(),
                        frozen_messages: VecDeque::new(),
                        abort_stream: None,
                        cancel_task: None,
                        broadcaster_id: None,
                        broadcaster_name: None,
                        emote_db: None,
                    };
                    println!("Inserting new chat instance: {}", stream_name);
                    self.chat_instances
                        .insert(stream_name.clone(), new_instance);

                    // TODO: Fix -- This if statement is unnecessary, we just insereted it so
                    // we know its in the map.
                    if let Some(instance) = self.chat_instances.get_mut(&stream_name) {
                        let (task, handle) = iced::Task::sip(
                            subscribe_to_chat_stream(format!("twitch://{}", stream_name)),
                            Message::HandleChatStreamOutput,
                            Message::ChatStreamError,
                        )
                        .abortable();
                        instance.abort_stream = Some(handle);
                        self.active_chat_instance = stream_name;
                        task
                    } else {
                        eprintln!("No stream instance found for {}", stream_name);
                        iced::Task::none()
                    }
                }
            }
            Message::ChatStreamError(result) => {
                if let Err(error_message) = result {
                    eprintln!("ChatStreamErorr: {}", error_message);
                    iced::Task::done(Message::Terminate)
                } else {
                    iced::Task::none()
                }
            }
            Message::UpdateAppConfig(config) => match config {
                None => iced::Task::done(Message::HandleError(String::from(
                    "Failed to initialize config.",
                ))),
                Some((api_config, app_config)) => {
                    self.api_config = Some(api_config);
                    self.app_config = Some(app_config);
                    iced::Task::none()
                }
            },
            Message::HandleChatStreamOutput(message) => match message {
                ChatStreamMessage::InitializeEmoteDb => {
                    let instance = self.active_chat_instance.clone();
                    iced::Task::perform(
                        stream::twitch::create_twitch_emote_database(instance.clone()),
                        |result| Message::AttachEmoteDatabase(instance, result),
                    )
                }
                ChatStreamMessage::SaveUserId(user_id) => {
                    self.user_id = Some(user_id);
                    iced::Task::none()
                }
                ChatStreamMessage::MessagePost(message) => {
                    let chat_frozen = self.chat_frozen;
                    match self.get_active_chat_instance_mut() {
                        Some(instance) => {
                            if chat_frozen {
                                instance.frozen_messages.push_back(message);
                                instance.frozen_messages.truncate_front(MAX_CHAT_MESSAGES);
                            } else {
                                instance.active_messages.push_back(message);
                                instance.active_messages.truncate_front(MAX_CHAT_MESSAGES);
                            }
                        }
                        None => {
                            eprintln!(
                                "Cannot post message: No chat instance for: \"{}\"",
                                self.active_chat_instance,
                            );
                        }
                    }
                    iced::Task::none()
                }
                ChatStreamMessage::SaveBroadcasterInfo(
                    broadcaster_id,
                    broadcaster_name,
                    cancel_token,
                ) => {
                    if let Some(chat_instance) = self.chat_instances.get_mut(&broadcaster_name) {
                        chat_instance.broadcaster_id = Some(broadcaster_id);
                        chat_instance.broadcaster_name = Some(broadcaster_name);
                        chat_instance.cancel_task = Some(cancel_token);
                        iced::Task::none()
                    } else {
                        iced::Task::done(Message::HandleError(format!(
                            "Cannot save broadcast info for broascaster: {}",
                            broadcaster_name
                        )))
                    }
                }
                ChatStreamMessage::MessageStreamInitState(stream_init_success) => {
                    self.subscribed_to_message_stream = stream_init_success;
                    iced::Task::none()
                }
                ChatStreamMessage::InitializeChatError(error_message) => {
                    iced::Task::done(Message::HandleError(error_message))
                }
            },
            Message::HandleError(error_message) => {
                // TODO: display the error in some modal.
                eprintln!("Erorr: {}", error_message);
                iced::Task::done(Message::Terminate)
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
            Message::CommandPaletteSelect => match self.command_palette_ctx.maybe_select() {
                Some(msg) => iced::Task::batch([
                    iced::Task::done(msg),
                    iced::Task::done(Message::CommandPaletteToggle(false, None)),
                ]),
                None => iced::Task::none(),
            },
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

                    if let Some(instance) = self.chat_instances.get(&self.active_chat_instance) {
                        if let (Some(broadcaster_id), Some(user_id)) =
                            (instance.broadcaster_id.as_ref(), self.user_id.as_ref())
                        {
                            let broadcaster_id = broadcaster_id.clone();
                            let user_id = user_id.clone();
                            iced::Task::perform(
                                (|| async move {
                                    stream::twitch::send_message(
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
                            eprintln!(
                                "Not sending message.. no broadcaster/user id found in state..."
                            );
                            iced::Task::none()
                        }
                    } else {
                        eprintln!(
                            "Not sending message.. no instance for active stream found in state..."
                        );
                        iced::Task::none()
                    }
                }
            }
            Message::SwitchActiveChat(active_chat) => {
                if self.chat_instances.contains_key(&active_chat) {
                    self.active_chat_instance = active_chat;
                }
                iced::Task::none()
            }
            Message::CommandPaletteToggle(enable_command_palette, starting_query) => {
                self.command_palette_ctx.active = enable_command_palette;
                if enable_command_palette {
                    if let Some(starting_query) = starting_query {
                        self.command_palette_ctx
                            .update_current_from_query(starting_query);
                    }
                    iced::widget::text_input::focus("command-palette-input")
                } else {
                    self.command_palette_ctx
                        .update_current_from_query(String::new());
                    iced::Task::done(Message::FocusChatArea)
                }
            }
            Message::FocusChatArea => iced::widget::text_input::focus("chat-message-input"),
            Message::HandleSpecialKey(key) => match key {
                iced::keyboard::key::Named::Escape => {
                    iced::Task::done(Message::CommandPaletteToggle(false, None))
                }
                arrow @ iced::keyboard::key::Named::ArrowUp
                | arrow @ iced::keyboard::key::Named::ArrowDown
                | arrow @ iced::keyboard::key::Named::Tab => {
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
                        if next_index >= self.command_palette_ctx.current_action.len() as i32 {
                            next_index = self.command_palette_ctx.current_action.len() as i32 - 1;
                        }
                        iced::Task::done(Message::CommandPaletteHighlightActionIndex(next_index))
                    } else {
                        iced::Task::none()
                    }
                }
                _ => iced::Task::none(),
            },
            Message::CommandPaletteHighlightActionIndex(index) => {
                self.command_palette_ctx.update_selected_query(index)
            }
            Message::CommandPaletteHandleAction(action, args) => {
                match action.as_str() {
                    "quit" => iced::Task::done(Message::Terminate),
                    // TODO: bind these action strings to some searchable symbol.
                    "open stream" => {
                        match args {
                            Some(args) => {
                                iced::Task::done(Message::OpenChatStream(args[0].clone()))
                            }
                            None => {
                                // TODO: this should probably not be fatal?
                                iced::Task::done(Message::HandleError(String::from(
                                    "No stream selected to open...",
                                )))
                            }
                        }
                    }
                    _ => iced::Task::done(Message::HandleError(format!(
                        "Unknown action: {}",
                        action
                    ))),
                }
            }
            Message::ChatViewportScroll(offset) => {
                let previously_frozen = self.chat_frozen;
                self.chat_frozen = offset.y > 0 as f32;
                if previously_frozen && !self.chat_frozen {
                    // Flush the frozen messages into the active messages.
                    match self.get_active_chat_instance_mut() {
                        Some(instance) => {
                            println!(
                                "Unfreezing the chat: Attaching {} frozen messages to chat.",
                                instance.frozen_messages.len()
                            );
                            instance
                                .active_messages
                                .append(&mut instance.frozen_messages);
                            assert!(instance.frozen_messages.is_empty());
                            instance.active_messages.truncate_front(MAX_CHAT_MESSAGES);
                        }
                        None => {}
                    }
                }
                iced::Task::none()
            }
            Message::AttachEmoteDatabase(chat_instance_id, emote_db) => match emote_db {
                crate::stream::base::EmoteDatabaseEnum::NoDatabase => {
                    eprintln!(
                        "[emote_db] No emote db initialized for chat instance: {}",
                        chat_instance_id
                    );
                    iced::Task::none()
                }
                crate::stream::base::EmoteDatabaseEnum::TwitchEmoteDatabase(db) => {
                    match self.get_chat_instance_mut(&chat_instance_id) {
                        None => {
                            eprintln!(
                                "No chat instance found w/ id ({}) to attach emote database to.",
                                chat_instance_id
                            );
                            iced::Task::none()
                        }
                        Some(instance) => {
                            instance.emote_db = Some(Box::new(db));
                            iced::Task::none()
                        }
                    }
                }
            },
            Message::Nothing(_) => iced::Task::none(),
        }
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        if let (Some(_), Some(_)) = (&self.api_config, &self.app_config) {
            let keyrelease_sub =
                iced::keyboard::on_key_release(|key, mods| match (key.as_ref(), mods) {
                    (iced::keyboard::Key::Named(k @ iced::keyboard::key::Named::Escape), _)
                    | (iced::keyboard::Key::Named(k @ iced::keyboard::key::Named::ArrowUp), _)
                    | (iced::keyboard::Key::Named(k @ iced::keyboard::key::Named::ArrowDown), _)
                    | (iced::keyboard::Key::Named(k @ iced::keyboard::key::Named::Tab), _) => {
                        Some(Message::HandleSpecialKey(k))
                    }
                    // TODO: Fix -- if focus is within an input, the command+P keybind will still
                    // output the p to the current input before evaluating the task generated here.
                    (iced::keyboard::Key::Character("p"), iced::keyboard::Modifiers::CTRL)
                    | (iced::keyboard::Key::Character("p"), iced::keyboard::Modifiers::COMMAND) => {
                        Some(Message::CommandPaletteToggle(true, None))
                    }
                    // TODO: Fix -- mapping to LOGO does not restrict the keys from being sent
                    // to the focused input.
                    (iced::keyboard::Key::Character("t"), iced::keyboard::Modifiers::LOGO)
                    | (iced::keyboard::Key::Character("n"), iced::keyboard::Modifiers::LOGO)
                    | (iced::keyboard::Key::Character("t"), iced::keyboard::Modifiers::CTRL)
                    | (iced::keyboard::Key::Character("n"), iced::keyboard::Modifiers::CTRL) => {
                        Some(Message::CommandPaletteToggle(
                            true,
                            Some(String::from("open stream: ")),
                        ))
                    }
                    (iced::keyboard::Key::Character("w"), iced::keyboard::Modifiers::CTRL)
                    | (iced::keyboard::Key::Character("w"), iced::keyboard::Modifiers::COMMAND) => {
                        Some(Message::CloseActiveChatStream)
                    }
                    _ => None,
                });
            // TODO: No need to batch this since it's only one subscription.
            iced::Subscription::batch([keyrelease_sub])
        } else {
            iced::Subscription::none()
        }
    }

    fn view(&self) -> iced::Element<Message> {
        let mut v = column![];
        if let Some(instance) = self.chat_instances.get(&self.active_chat_instance) {
            for msg in &instance.active_messages {
                v = v.push(
                    iced::widget::container(
                        rich_text![
                            span(format!("{}: ", &msg.username))
                                .color(color!(0x000000))
                                .font(Font {
                                    weight: font::Weight::Bold,
                                    ..Font::default()
                                }),
                            span(&msg.message).color(color!(0x212121))
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
        }

        let stream_tab_line_height = 30;
        let stream_tab = |stream_name: String, active| {
            let mut tab = iced::widget::button(iced::widget::text(stream_name.clone()))
                .on_press(Message::SwitchActiveChat(stream_name.clone()));
            if active {
                tab = tab.style(|theme, status| {
                    iced::widget::button::primary(theme, status).with_background(color!(0x193827))
                });
            } else {
                tab = tab.style(|theme, status| {
                    iced::widget::button::primary(theme, status).with_background(color!(0x132C1F))
                });
            }

            tab
        };

        let mut stream_tab_row = row![];
        for (stream_name, _) in &self.chat_instances {
            // TODO: When an inactive tab is clicked on, switch to that tab.
            let active_tab = &self.active_chat_instance == stream_name;
            stream_tab_row = stream_tab_row.push(stream_tab(stream_name.clone(), active_tab));
        }

        let scrollable_chat_area = iced::widget::scrollable(v)
            .width(iced::Fill)
            .height(iced::Fill)
            .style(|theme, status| {
                let mut style = iced::widget::scrollable::default(theme, status);
                style.container =
                    style
                        .container
                        .background(iced::Background::Gradient(make_gradient(
                            color!(0xD9CEBD),
                            color!(0xEDE4D5),
                            0 as f32,
                        )));
                style
            })
            .on_scroll(|viewport| Message::ChatViewportScroll(viewport.absolute_offset()))
            .anchor_bottom();

        let base_ui = column![
            iced::widget::container(stream_tab_row)
                .height(stream_tab_line_height)
                .width(iced::Fill)
                .style(|_theme| { iced::widget::container::background(color!(0x132C1F)) }),
            scrollable_chat_area,
            row![
                iced::widget::text_input("Send chat message...", self.input_message.as_str())
                    .id("chat-message-input")
                    .on_input(Message::InputMessageChanged)
                    .on_submit(Message::SendInputMessage)
                    .width(iced::Fill)
                    .padding([20, 20])
                    .style(|theme, status| {
                        let color = color!(0xccc2b3);
                        let mut style = iced::widget::text_input::default(theme, status);
                        style.border.radius = iced::border::left(0.0);
                        style.border.color = color;
                        style.value = color!(0x000000);
                        style.background = iced::Background::Color(color);
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
                        let mut style = iced::widget::button::primary(theme, status);
                        style.border.radius = iced::border::left(0.0);
                        style.background = Some(iced::Background::Gradient(make_gradient(
                            color!(0xecb073),
                            color!(0xEABEC0),
                            0 as f32,
                        )));
                        style
                    })
                    .padding(20)
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
                .style(if self.command_palette_ctx.current_action.is_empty() {
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
        // TODO: visibility -- bold/highlight the parts of the option that match the query.
        for (i, option) in self.command_palette_ctx.current_action.iter().enumerate() {
            let active = self.command_palette_ctx.selected_index == i as i32;
            let is_last = i + 1 == self.command_palette_ctx.current_action.len();
            let entry = iced::widget::container(
                iced::widget::text(option).size(14).color(color!(0x000000)),
            )
            .width(iced::Fill)
            .padding([5, 10])
            .style(move |_theme| AppStyle::command_palette_result_entry_container(active, is_last));
            results = results.push(entry);
        }
        results.into()
    }

    fn get_chat_instance_mut(&mut self, chat_instance_id: &str) -> Option<&mut ChatInstance> {
        if self.chat_instances.contains_key(chat_instance_id) {
            match self.chat_instances.get_mut(chat_instance_id) {
                Some(instance) => Some(instance),
                None => None,
            }
        } else {
            None
        }
    }

    fn get_active_chat_instance_mut(&mut self) -> Option<&mut ChatInstance> {
        // TODO: opt - I don't like this clone...
        let id = self.active_chat_instance.clone();
        self.get_chat_instance_mut(&id)
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

    fn command_palette_result_entry_container(
        active: bool,
        is_last: bool,
    ) -> iced::widget::container::Style {
        let mut style = if active {
            iced::widget::container::background(color!(0xada292))
        } else {
            iced::widget::container::background(color!(0xbab0a2))
        };
        if is_last {
            style.border.radius.bottom_left = 5.0;
            style.border.radius.bottom_right = 5.0;
        }
        style.shadow = iced::Shadow {
            color: color!(0x000000, 0.2),
            blur_radius: 10 as f32,
            offset: iced::Vector::new(0 as f32, 5 as f32),
        };
        style
    }

    fn command_palette_text_input_default(
        theme: &iced::widget::Theme,
        status: iced::widget::text_input::Status,
    ) -> iced::widget::text_input::Style {
        let mut style = iced::widget::text_input::default(theme, status);
        let clr = color!(0xccc2b3);
        style.background = iced::Background::Color(clr);
        style.border.color = clr;
        style.value = color!(0x000000);
        style
    }

    fn command_palette_text_input_with_results(
        theme: &iced::widget::Theme,
        status: iced::widget::text_input::Status,
    ) -> iced::widget::text_input::Style {
        let mut style = Self::command_palette_text_input_default(theme, status);
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
        let mut style = Self::command_palette_text_input_default(theme, status);
        style.border.radius.top_right = 5.0;
        style.border.radius.top_left = 5.0;
        style.border.radius.bottom_right = 5.0;
        style.border.radius.bottom_left = 5.0;
        style
    }
}

fn make_gradient(clr1: iced::Color, clr2: iced::Color, angle_radians: f32) -> iced::Gradient {
    let linear = iced::gradient::Linear {
        angle: iced::Radians(angle_radians),
        stops: [
            Some(iced::gradient::ColorStop {
                offset: 0 as f32,
                color: clr1,
            }),
            Some(iced::gradient::ColorStop {
                offset: 1 as f32,
                color: clr2,
            }),
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    };
    iced::Gradient::Linear(linear)
}

struct CommandPalette {
    // k-v pair of actions that map to messages that should be emitted
    // when the action is selected.
    // TODO: Find a way to use a hash map that stores the enum message
    // that should be emitted when the action is selected.
    actions: HashSet<String>,
    // The current relevant actions based on the user's query.
    current_action: Vec<String>,
    current_action_arg: String,
    query: String,
    // If true, the command palette is in focus and being used.
    active: bool,
    // The option in the commant palette list that is highlighted.
    selected_index: i32,
}

impl CommandPalette {
    fn new() -> Self {
        Self {
            actions: std::collections::HashSet::from([
                // Quits the application.
                String::from("quit"),
                // Opens the stream.
                String::from("open stream"),
            ]),
            current_action: vec![],
            current_action_arg: String::new(),
            query: String::new(),
            active: false,
            selected_index: -1,
        }
    }

    // Try to select the current highlighted option.
    // If no option is highlighted, nothing is done.
    fn maybe_select(&self) -> Option<Message> {
        if self.selected_index >= self.current_action.len() as i32 || self.selected_index < 0 {
            None
        } else {
            let selected_key = &self.current_action[self.selected_index as usize];
            Some(Message::CommandPaletteHandleAction(
                selected_key.clone(),
                if self.current_action_arg.is_empty() {
                    None
                } else {
                    Some(vec![self.current_action_arg.clone()])
                },
            ))
        }
    }

    // Updates the query to the action at the given `index`.
    // The query will be updated to contain the full text of the selected action
    // if it is not fully typed out in the command palette text input.
    fn update_selected_query(&mut self, index: i32) -> iced::Task<Message> {
        let mut index = index;
        if index < -1 {
            index = -1;
        } else if index >= self.current_action.len() as i32 {
            index = self.current_action.len() as i32 - 1;
        }
        self.selected_index = index;
        if index >= 0 {
            if let Some(q) = self.current_action.get(index as usize) {
                // If the query already contains the action and also has arguments,
                // do not override the query.
                let parts = self.query.splitn(2, ":").collect::<Vec<_>>();
                if parts.len() < 2 {
                    self.query = format!("{}: ", q);
                }
            }
            iced::widget::text_input::move_cursor_to_end("command-palette-input")
        } else {
            iced::Task::none()
        }
    }

    fn update_current_from_query(&mut self, query: String) {
        let mut current_actions = vec![];
        let query = query.trim_start();

        // query has 2 parts-- first part is before a colon
        // (if there is one), and second part is after the colon.
        // The part before the colon is the action and the part
        // after the colon are the arguments for the action.

        let parts = query.splitn(2, ":").collect::<Vec<&str>>();
        let action = if parts.is_empty() { "" } else { parts[0] };

        if !action.is_empty() {
            for k in &self.actions {
                if k.starts_with(action) {
                    current_actions.push(k.clone());
                }
            }
        }

        // If any action is available that matches the query, and the
        // selected index is on no actions, auto select the first action.
        if !current_actions.is_empty() && self.selected_index == -1 {
            self.selected_index = 0;
        }
        self.current_action = current_actions;
        self.current_action_arg = if parts.len() == 2 {
            String::from(parts[1].trim())
        } else {
            String::new()
        };
        self.query = String::from(query);
    }
}

async fn create_message_stream(url: &str) -> std::result::Result<Box<dyn MessageStream>, String> {
    let url_parts = url.splitn(2, "://").collect::<Vec<_>>();
    if url_parts.len() != 2 {
        return Err(String::from("Invalid url, no protocol found."));
    }

    let (protocol, path) = (url_parts[0], url_parts[1]);
    match protocol {
        "file" => Ok(Box::new(stream::csv::CsvMessageStream::new_from_file(path))),
        "twitch" => Ok(Box::new(
            stream::twitch::TwitchMessageStream::new_for_stream(path).await,
        )),
        _ => Err(String::from(format!("Invalid protocol: {}", protocol))),
    }
}

fn subscribe_to_chat_stream(url: String) -> impl iced::task::Straw<(), ChatStreamMessage, String> {
    iced::task::sipper(async move |mut output| {
        let stream = create_message_stream(url.as_str()).await;
        if let Err(e) = stream {
            // TODO: Return the error instead of emitting the error message...
            output.send(ChatStreamMessage::InitializeChatError(e)).await;
        } else if let Ok(mut stream) = stream {
            if let Some((broadcaster_id, broadcaster_name, user_id, cancel_token)) =
                stream.get_broadcaster_and_user_id()
            {
                if let Some(cancel_token) = cancel_token {
                    output
                        .send(ChatStreamMessage::SaveBroadcasterInfo(
                            broadcaster_id,
                            broadcaster_name,
                            cancel_token,
                        ))
                        .await;
                    output.send(ChatStreamMessage::SaveUserId(user_id)).await;
                    output.send(ChatStreamMessage::InitializeEmoteDb).await;
                } else {
                    output
                        .send(ChatStreamMessage::InitializeChatError(String::from(
                            "Failed to get cancellation token.",
                        )))
                        .await;
                }
            }
            output
                .send(ChatStreamMessage::MessageStreamInitState(true))
                .await;
            loop {
                let user_message = stream.next_message().await;
                if let Some(user_message) = user_message {
                    output
                        .send(ChatStreamMessage::MessagePost(user_message))
                        .await;
                }
                tokio::time::sleep(std::time::Duration::from_nanos(100)).await;
            }
        }
        Ok(())
    })
}
