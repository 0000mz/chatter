use iced::widget::{column, rich_text, span};
use iced::{Font, color, font};
use std::collections::VecDeque;
use std::vec::Vec;

fn main() -> iced::Result {
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
