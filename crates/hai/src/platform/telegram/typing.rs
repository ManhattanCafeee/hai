//! Telegram's typing indicator: the Bot API's own action, repeated for as long as the run lasts.

use std::time::Duration;

use teloxide::{
    Bot,
    prelude::Requester,
    types::{ChatAction, ChatId as TgChatId},
};
use tokio::task::JoinHandle;

use crate::agent::channel::TypingGuard;

/// Telegram's action reaches a client as about five seconds of "typing"; repeating under that keeps
/// it unbroken between ticks.
const TYPING_REPEAT: Duration = Duration::from_millis(4_500);

/// One live typing indicator: the task repeating the action, and the guard that owns it. Dropping
/// this ends the indicator.
pub(super) struct TelegramTyping(JoinHandle<()>);

impl TelegramTyping {
    /// The first action goes out at once, the rest on the cadence. A failed action is logged and the
    /// next tick tries again: the indicator is decoration, and the run it covers is not affected.
    pub(super) fn start(bot: Bot, chat_id: TgChatId) -> Self {
        Self(tokio::spawn(async move {
            let mut interval = tokio::time::interval(TYPING_REPEAT);
            loop {
                interval.tick().await;
                if let Err(err) = bot.send_chat_action(chat_id, ChatAction::Typing).await {
                    tracing::warn!(chat_id = chat_id.0, error = %err, "typing indicator skipped");
                }
            }
        }))
    }
}

impl TypingGuard for TelegramTyping {}

impl Drop for TelegramTyping {
    fn drop(&mut self) {
        self.0.abort();
    }
}
