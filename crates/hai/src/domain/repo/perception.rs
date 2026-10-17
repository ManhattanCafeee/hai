use sqlx::PgPool;

use crate::{
    domain::{
        model::Perception,
        repo::vector,
        vo::{AttachmentParser, PerceptionId, Subject, SubjectKind},
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct PerceptionRepo {
    pool: PgPool,
}

impl PerceptionRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Exact cache lookup on (subject_kind, subject, parser, prompt); `None` matches the
    /// `prompt IS NULL` row.
    pub async fn find(
        &self,
        subject: &Subject,
        parser: AttachmentParser,
        focus: Option<&str>,
    ) -> Result<Option<Perception>> {
        sqlx::query_as::<_, Perception>(
            "SELECT * FROM perception \
             WHERE subject_kind = $1 AND subject = $2 AND parser = $3 \
               AND (prompt = $4 OR (prompt IS NULL AND $4 IS NULL)) \
             LIMIT 1",
        )
        .bind(subject.kind.name())
        .bind(&subject.value)
        .bind(parser.name())
        .bind(focus)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// One round trip for many subjects of the same kind.
    pub async fn find_by_subjects(
        &self,
        kind: SubjectKind,
        subjects: &[String],
    ) -> Result<Vec<Perception>> {
        if subjects.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as::<_, Perception>(
            "SELECT * FROM perception WHERE subject_kind = $1 AND subject = ANY($2)",
        )
        .bind(kind.name())
        .bind(subjects)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Writes the vector the row is found by; the caller generates it and decides what a failure
    /// means.
    pub async fn store_embedding(&self, id: PerceptionId, embedding: &[f32]) -> Result<()> {
        vector::upsert_embedding_vec(
            &self.pool,
            vector::EmbeddingTable::Perception,
            id.0,
            embedding,
        )
        .await
    }

    /// Returns the row as it is after the write: a caller handed the row it read would be given the
    /// content this call replaced.
    pub async fn update_content(
        &self,
        id: PerceptionId,
        content: &str,
    ) -> Result<Option<Perception>> {
        sqlx::query_as::<_, Perception>(
            "UPDATE perception SET content = $2 WHERE id = $1 RETURNING *",
        )
        .bind(id.0)
        .bind(content)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn create(
        &self,
        subject: &Subject,
        parser: AttachmentParser,
        focus: Option<&str>,
        content: &str,
    ) -> Result<Perception> {
        sqlx::query_as::<_, Perception>(
            "INSERT INTO perception (id, subject_kind, subject, parser, prompt, content) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(subject.kind.name())
        .bind(&subject.value)
        .bind(parser.name())
        .bind(focus)
        .bind(content)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }
}
