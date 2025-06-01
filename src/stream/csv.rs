use crate::stream::base::EmoteDatabase;
use crate::stream::base::MessageStream;
use crate::stream::base::UserMessage;

use async_trait::async_trait;
use std::collections::VecDeque;

pub struct CsvMessageStream {
    messages: VecDeque<UserMessage>,
}

impl CsvMessageStream {
    pub fn new_from_file(filepath: &str) -> Self {
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
            let broadcast_id = String::from("placeholder_id");
            let broadcaster_name = String::from("placeholder_name");
            user_messages.push(UserMessage::new(
                parts[0], // username as user id
                parts[0],
                parts[1],
                broadcast_id.as_ref(),
                broadcaster_name.as_ref(),
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
    fn get_broadcaster_and_user_id(
        &self,
    ) -> Option<(
        String,
        String,
        String,
        Option<tokio_util::sync::CancellationToken>,
    )> {
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

#[derive(Clone)]
struct CsvEmoteDatabase;

impl EmoteDatabase for CsvEmoteDatabase {}
