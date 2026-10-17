use crate::domain::vo::{ChannelKey, ChatId, RepeatInterval, ScheduledTaskId};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ScheduledTask {
    pub id: ScheduledTaskId,
    pub bot_id: ChannelKey,
    pub chat_id: ChatId,
    pub description: String,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub fire_at: jiff::Timestamp,
    pub every_secs: Option<RepeatInterval>,
    pub is_active: bool,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
}

impl ScheduledTask {
    /// Next fire strictly after `now`; missed slots collapse into a single catch-up fire.
    /// `None` for a task that does not repeat.
    pub fn next_fire_after(&self, now: jiff::Timestamp) -> Option<jiff::Timestamp> {
        let step = self.every_secs?.as_duration();
        let mut next = self.fire_at;
        while next <= now {
            next += step;
        }
        Some(next)
    }
}
