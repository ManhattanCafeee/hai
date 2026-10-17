use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    domain::{
        model::{Topic, TopicNote, TopicStatus},
        repo::{lexical, vector},
        vo::{ChatId, MessageId, TimeRange, TopicId},
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct TopicRepo {
    pool: PgPool,
}

impl TopicRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Opens a topic with the messages that opened it and the vector retrieval finds it by, in one
    /// transaction.
    pub async fn open(
        &self,
        chat_id: ChatId,
        title: &str,
        summary: &str,
        embedding: &[f32],
        message_ids: &[MessageId],
    ) -> Result<Topic> {
        let mut tx = self.pool.begin().await?;
        let topic = insert_topic(&mut tx, chat_id, title, summary, embedding).await?;
        link_in_tx(&mut tx, topic.id, message_ids).await?;
        tx.commit().await?;
        Ok(topic)
    }

    /// Assigns the messages and refreshes the time range in one transaction; the messages'
    /// interaction status is left alone. `false` when the topic is not this chat's: the scope is a
    /// statement, not the caller's memory.
    pub async fn link_messages(
        &self,
        chat_id: ChatId,
        topic_id: TopicId,
        message_ids: &[MessageId],
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let ours =
            sqlx::query("UPDATE topic SET updated_at = now() WHERE id = $1 AND chat_id = $2")
                .bind(topic_id.0)
                .bind(chat_id.0)
                .execute(&mut *tx)
                .await?;
        if ours.rows_affected() == 0 {
            return Ok(false);
        }
        link_in_tx(&mut tx, topic_id, message_ids).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Adds the note only when the topic is this chat's; `None` means it is not.
    pub async fn add_note(
        &self,
        chat_id: ChatId,
        topic_id: TopicId,
        text: &str,
        message_ids: &[MessageId],
    ) -> Result<Option<TopicNote>> {
        sqlx::query_as::<_, TopicNote>(
            "INSERT INTO topic_note (topic_id, text, message_ids) \
             SELECT $1, $2, $3 WHERE EXISTS (SELECT 1 FROM topic WHERE id = $1 AND chat_id = $4) \
             RETURNING *",
        )
        .bind(topic_id.0)
        .bind(text)
        .bind(message_ids)
        .bind(chat_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn notes(
        &self,
        topic_id: TopicId,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<TopicNote>> {
        sqlx::query_as::<_, TopicNote>(
            "SELECT * FROM topic_note WHERE topic_id = $1 ORDER BY id LIMIT $2 OFFSET $3",
        )
        .bind(topic_id.0)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn notes_count(&self, topic_id: TopicId) -> Result<i64> {
        sqlx::query_scalar("SELECT count(*) FROM topic_note WHERE topic_id = $1")
            .bind(topic_id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// `None` keeps the current value; a closed topic is still writable. Title, summary and the
    /// vector go in one statement, so retrieval never matches a meaning the row no longer has.
    pub async fn update_fields(
        &self,
        chat_id: ChatId,
        topic_id: TopicId,
        title: Option<&str>,
        summary: Option<&str>,
        embedding: &[f32],
    ) -> Result<Option<Topic>> {
        sqlx::query_as::<_, Topic>(
            "UPDATE topic SET title = COALESCE($3, title), summary = COALESCE($4, summary), \
             embedding = $5::vector, updated_at = now() \
             WHERE id = $1 AND chat_id = $2 RETURNING *",
        )
        .bind(topic_id.0)
        .bind(chat_id.0)
        .bind(title)
        .bind(summary)
        .bind(vector::vec_to_pgstring(embedding))
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Closes the topic: status and time range only, `summary` stays as it is. `None` when the topic
    /// is not this chat's, in which case nothing is written.
    pub async fn close(&self, chat_id: ChatId, topic_id: TopicId) -> Result<Option<Topic>> {
        let mut tx = self.pool.begin().await?;
        let topic = sqlx::query_as::<_, Topic>(
            "UPDATE topic SET status = $3, closed_at = $4, updated_at = $4 \
             WHERE id = $1 AND chat_id = $2 RETURNING *",
        )
        .bind(topic_id.0)
        .bind(chat_id.0)
        .bind(TopicStatus::Closed.as_str())
        .bind(jiff_sqlx::Timestamp::from(jiff::Timestamp::now()))
        .fetch_optional(&mut *tx)
        .await?;
        let Some(topic) = topic else {
            return Ok(None);
        };
        sync_topic_times(&mut tx, topic_id).await?;
        tx.commit().await?;
        Ok(Some(topic))
    }

    pub async fn by_id(&self, chat_id: ChatId, id: TopicId) -> Result<Option<Topic>> {
        sqlx::query_as::<_, Topic>("SELECT * FROM topic WHERE id = $1 AND chat_id = $2")
            .bind(id.0)
            .bind(chat_id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn by_ids(&self, ids: &[TopicId]) -> Result<Vec<Topic>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as::<_, Topic>("SELECT * FROM topic WHERE id = ANY($1)")
            .bind(ids)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn active_by_chat(&self, chat_id: ChatId) -> Result<Vec<Topic>> {
        sqlx::query_as::<_, Topic>(
            "SELECT * FROM topic WHERE chat_id = $1 AND status = $2 ORDER BY last_active_at DESC",
        )
        .bind(chat_id.0)
        .bind(TopicStatus::Active.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Nearest topics by meaning; every topic stays retrievable, closed or not.
    pub async fn search_by_embedding(
        &self,
        chat_id: ChatId,
        query: &[f32],
        limit: i64,
    ) -> Result<Vec<(TopicId, f64)>> {
        let rows = vector::search_chat_embedding_vec(
            &self.pool,
            vector::ChatTable::Topic,
            query,
            chat_id,
            None,
            limit,
        )
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, distance)| (TopicId(id), distance))
            .collect())
    }

    pub async fn by_chat_time(
        &self,
        chat_id: ChatId,
        range: TimeRange,
        limit: i64,
    ) -> Result<Vec<Topic>> {
        let sql = format!(
            "SELECT * FROM topic WHERE chat_id = $1 {} ORDER BY last_active_at DESC LIMIT $4",
            time_window("last_active_at", 2)
        );
        let query = sqlx::query_as::<_, Topic>(sqlx::AssertSqlSafe(sql.as_str())).bind(chat_id.0);
        range
            .bind(query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn search_by_keywords(
        &self,
        chat_id: ChatId,
        keywords: &[String],
        range: TimeRange,
        limit: i64,
    ) -> Result<Vec<Topic>> {
        let patterns = lexical::patterns(keywords);
        let query = sqlx::query_as::<_, Topic>(sqlx::AssertSqlSafe(keyword_search_sql().as_str()))
            .bind(chat_id.0);
        range
            .bind(query)
            .bind(&patterns)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Removes the topic with its notes, and unlinks its messages; `false` when it is not this
    /// chat's, which the caller reports as a missing topic rather than a silent success.
    pub async fn delete(&self, chat_id: ChatId, topic_id: TopicId) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let ours = sqlx::query("DELETE FROM topic WHERE id = $1 AND chat_id = $2")
            .bind(topic_id.0)
            .bind(chat_id.0)
            .execute(&mut *tx)
            .await?;
        if ours.rows_affected() == 0 {
            return Ok(false);
        }
        sqlx::query("DELETE FROM topic_note WHERE topic_id = $1")
            .bind(topic_id.0)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE message SET topic_id = NULL WHERE topic_id = $1")
            .bind(topic_id.0)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}

/// Inserts a topic with the vector it is retrieved by; the caller owns the transaction.
async fn insert_topic(
    tx: &mut Transaction<'_, Postgres>,
    chat_id: ChatId,
    title: &str,
    summary: &str,
    embedding: &[f32],
) -> Result<Topic> {
    let now = jiff_sqlx::Timestamp::from(jiff::Timestamp::now());
    sqlx::query_as::<_, Topic>(
        "INSERT INTO topic (id, chat_id, title, summary, status, started_at, last_active_at, \
         created_at, updated_at, embedding) \
         VALUES ($1, $2, $3, $4, $5, $6, $6, $6, $6, $7::vector) RETURNING *",
    )
    .bind(Uuid::now_v7())
    .bind(chat_id.0)
    .bind(title)
    .bind(summary)
    .bind(TopicStatus::Active.as_str())
    .bind(now)
    .bind(vector::vec_to_pgstring(embedding))
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

/// Assigns the messages to the topic and refreshes its time range; the caller owns the transaction
/// and has already established that the topic is the chat's.
async fn link_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    topic_id: TopicId,
    message_ids: &[MessageId],
) -> Result<()> {
    if !message_ids.is_empty() {
        sqlx::query("UPDATE message SET topic_id = $2 WHERE id = ANY($1)")
            .bind(message_ids)
            .bind(topic_id.0)
            .execute(&mut **tx)
            .await?;
    }
    sync_topic_times(tx, topic_id).await
}

/// Refreshes the first/last message id and the time range from this topic's messages, inside the
/// caller's transaction. The range uses `COALESCE(sent_at, created_at)`; a topic without messages
/// keeps its current values.
async fn sync_topic_times(tx: &mut Transaction<'_, Postgres>, topic_id: TopicId) -> Result<()> {
    sqlx::query(
        "UPDATE topic SET \
             first_message_id = m.min_id, last_message_id = m.max_id, \
             started_at = m.min_at, last_active_at = m.max_at \
         FROM (SELECT min(id) AS min_id, max(id) AS max_id, \
                      min(COALESCE(sent_at, created_at)) AS min_at, \
                      max(COALESCE(sent_at, created_at)) AS max_at \
               FROM message WHERE topic_id = $1) m \
         WHERE topic.id = $1 AND m.min_id IS NOT NULL",
    )
    .bind(topic_id.0)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The lexical leg's statement: the keyword patterns are bound at `$4`.
fn keyword_search_sql() -> String {
    format!(
        "SELECT t.* FROM topic t WHERE t.chat_id = $1 {} \
         AND {} \
         ORDER BY {}, \
                  t.last_active_at DESC LIMIT $5",
        time_window("t.last_active_at", 2),
        lexical::match_predicate(&["t.title", "t.summary"], 4),
        lexical::match_rank(&["t.title", "t.summary"], 4),
    )
}

/// The time window a listing narrows by, over `column`: the bounds are the two placeholders from
/// `first` on, and a NULL bound means unbounded on that side.
fn time_window(column: &str, first: usize) -> String {
    format!(
        "AND (${first}::timestamptz IS NULL OR {column} >= ${first}) \
         AND (${next}::timestamptz IS NULL OR {column} <= ${next})",
        next = first + 1,
    )
}
