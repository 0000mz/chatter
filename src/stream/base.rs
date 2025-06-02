use async_trait::async_trait;

#[derive(Clone)]
pub struct UserMessage {
    pub user_id: String,
    pub username: String,
    pub message: String,
    pub broadcast_id: String,
    pub broadcaster_name: String,
    pub timestamp: std::time::Instant,
}

impl UserMessage {
    pub fn new(
        user_id: &str,
        username: &str,
        message: &str,
        broadcast_id: &str,
        broadcaster_name: &str,
        timestamp: std::time::Instant,
    ) -> Self {
        UserMessage {
            user_id: user_id.into(),
            username: username.into(),
            message: message.into(),
            broadcast_id: broadcast_id.into(),
            broadcaster_name: broadcaster_name.into(),
            timestamp,
        }
    }
}

#[async_trait]
pub trait MessageStream: Send + Sync {
    async fn next_message(&mut self) -> Option<UserMessage>;
    fn get_broadcaster_and_user_id(
        &self,
    ) -> Option<(
        String,
        String,
        String,
        Option<tokio_util::sync::CancellationToken>,
    )>;
}

pub trait EmoteDatabase {
    // Returns the emote image handle associated with the string `s`.
    // Returns None if no emote is associated with the string.
    fn get_emote(&self, s: &str) -> Option<&iced::widget::image::Handle>;
}

#[derive(Clone)]
pub enum EmoteDatabaseEnum {
    TwitchEmoteDatabase(crate::stream::twitch::TwitchEmoteDatabase),
    NoDatabase,
}
