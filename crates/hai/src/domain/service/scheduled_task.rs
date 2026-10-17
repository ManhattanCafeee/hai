use crate::{
    domain::{
        model::ScheduledTask,
        repo::Repos,
        vo::{ChannelKey, ChatEntry, ChatId, RepeatInterval, ScheduledTaskId},
    },
    error::{AppError, Result},
};

#[derive(Debug, Clone)]
pub struct ScheduledTaskService {
    repos: Repos,
}

impl ScheduledTaskService {
    pub fn new(repos: Repos) -> Self {
        Self { repos }
    }

    pub async fn create(
        &self,
        bot_id: &ChannelKey,
        chat_id: ChatId,
        description: &str,
        fire_at: jiff::Timestamp,
        every_secs: Option<RepeatInterval>,
    ) -> Result<ScheduledTask> {
        self.repos
            .scheduled_task
            .create(bot_id, chat_id, description, fire_at, every_secs)
            .await
    }

    pub async fn list_active(
        &self,
        bot_id: &ChannelKey,
        chat_id: ChatId,
    ) -> Result<Vec<ScheduledTask>> {
        self.repos
            .scheduled_task
            .list_active_by_chat(bot_id, chat_id)
            .await
    }

    pub async fn list_all(
        &self,
        bot_id: &ChannelKey,
        chat_id: ChatId,
    ) -> Result<Vec<ScheduledTask>> {
        self.repos
            .scheduled_task
            .list_all_by_chat(bot_id, chat_id)
            .await
    }

    /// Soft cancel: the row stays in the table.
    pub async fn cancel(
        &self,
        bot_id: &ChannelKey,
        chat_id: ChatId,
        id: ScheduledTaskId,
    ) -> Result<()> {
        let affected = self
            .repos
            .scheduled_task
            .deactivate(bot_id, chat_id, id)
            .await?;
        if affected == 0 {
            return Err(AppError::message(format!("Task not found: {id}")));
        }
        Ok(())
    }

    pub async fn due(
        &self,
        bot_id: &ChannelKey,
        now: jiff::Timestamp,
    ) -> Result<Vec<ScheduledTask>> {
        self.repos.scheduled_task.due(bot_id, now).await
    }

    /// Fires one due task: the change and the advanced clock land together.
    pub async fn fire(
        &self,
        task: &ScheduledTask,
        entry: &ChatEntry,
        now: jiff::Timestamp,
    ) -> Result<()> {
        self.repos.scheduled_task.fire(task, entry, now).await
    }
}
