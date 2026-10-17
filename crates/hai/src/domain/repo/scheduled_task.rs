use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    domain::{
        model::ScheduledTask,
        repo::ConversationRecordRepo,
        vo::{ChannelKey, ChatEntry, ChatId, RepeatInterval, ScheduledTaskId},
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct ScheduledTaskRepo {
    pool: PgPool,
}

impl ScheduledTaskRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        bot_id: &ChannelKey,
        chat_id: ChatId,
        description: &str,
        fire_at: jiff::Timestamp,
        every_secs: Option<RepeatInterval>,
    ) -> Result<ScheduledTask> {
        let id = ScheduledTaskId::from(Uuid::now_v7());
        sqlx::query_as::<_, ScheduledTask>(
            "INSERT INTO scheduled_task (id, bot_id, chat_id, description, fire_at, every_secs) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
        )
        .bind(id)
        .bind(bot_id)
        .bind(chat_id.0)
        .bind(description)
        .bind(jiff_sqlx::Timestamp::from(fire_at))
        .bind(every_secs)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_active_by_chat(
        &self,
        bot_id: &ChannelKey,
        chat_id: ChatId,
    ) -> Result<Vec<ScheduledTask>> {
        self.list_by_chat(bot_id, chat_id, TaskListing::ActiveOnly)
            .await
    }

    pub async fn list_all_by_chat(
        &self,
        bot_id: &ChannelKey,
        chat_id: ChatId,
    ) -> Result<Vec<ScheduledTask>> {
        self.list_by_chat(bot_id, chat_id, TaskListing::EveryTask)
            .await
    }

    /// The one statement both chat listings run, differing only in whether dead tasks show.
    async fn list_by_chat(
        &self,
        bot_id: &ChannelKey,
        chat_id: ChatId,
        listing: TaskListing,
    ) -> Result<Vec<ScheduledTask>> {
        sqlx::query_as::<_, ScheduledTask>(sqlx::AssertSqlSafe(list_sql(listing).as_str()))
            .bind(bot_id)
            .bind(chat_id.0)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn deactivate(
        &self,
        bot_id: &ChannelKey,
        chat_id: ChatId,
        id: ScheduledTaskId,
    ) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE scheduled_task SET is_active = FALSE, updated_at = now() \
             WHERE id = $1 AND bot_id = $2 AND chat_id = $3",
        )
        .bind(id)
        .bind(bot_id)
        .bind(chat_id.0)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn due(
        &self,
        bot_id: &ChannelKey,
        now: jiff::Timestamp,
    ) -> Result<Vec<ScheduledTask>> {
        sqlx::query_as::<_, ScheduledTask>(
            "SELECT * FROM scheduled_task \
             WHERE bot_id = $1 AND is_active AND fire_at <= $2 \
             ORDER BY fire_at",
        )
        .bind(bot_id)
        .bind(jiff_sqlx::Timestamp::from(now))
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Appends the change and advances the clock in one transaction: a task that fired must not be
    /// due again because the second write did not land.
    pub async fn fire(
        &self,
        task: &ScheduledTask,
        entry: &ChatEntry,
        now: jiff::Timestamp,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        ConversationRecordRepo::append_entry_in_tx(&mut tx, task.chat_id, entry).await?;
        match task.next_fire_after(now) {
            Some(next) => {
                sqlx::query(
                    "UPDATE scheduled_task SET fire_at = $2, updated_at = now() WHERE id = $1",
                )
                .bind(task.id)
                .bind(jiff_sqlx::Timestamp::from(next))
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query(
                    "UPDATE scheduled_task SET is_active = FALSE, updated_at = now() WHERE id = $1",
                )
                .bind(task.id)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
}

/// Whether a chat listing shows dead tasks too.
enum TaskListing {
    ActiveOnly,
    EveryTask,
}

/// The statement both chat listings run, differing only in the `is_active` predicate.
fn list_sql(listing: TaskListing) -> String {
    let active = match listing {
        TaskListing::ActiveOnly => " AND is_active",
        TaskListing::EveryTask => "",
    };
    format!(
        "SELECT * FROM scheduled_task \
         WHERE bot_id = $1 AND chat_id = $2{active} \
         ORDER BY fire_at"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_active_listing_narrows_to_live_tasks() {
        assert_eq!(
            list_sql(TaskListing::ActiveOnly),
            "SELECT * FROM scheduled_task \
             WHERE bot_id = $1 AND chat_id = $2 AND is_active ORDER BY fire_at"
        );
        assert_eq!(
            list_sql(TaskListing::EveryTask),
            "SELECT * FROM scheduled_task \
             WHERE bot_id = $1 AND chat_id = $2 ORDER BY fire_at"
        );
    }
}
