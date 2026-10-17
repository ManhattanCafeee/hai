use crate::domain::vo::{AttachmentParser, PerceptionId, SubjectKind};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Perception {
    pub id: PerceptionId,

    /// Which kind of subject this row is cached against; the column value is `SubjectKind`'s name.
    pub subject_kind: SubjectKind,
    /// The subject itself: a `<platform>:<file_unique_id>` handle or a URL.
    pub subject: String,
    pub parser: AttachmentParser,
    /// Analysis focus; the column is `prompt`. `None` is the base transcription, `Some` a
    /// targeted reading.
    #[sqlx(rename = "prompt")]
    pub focus: Option<String>,
    pub content: String,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
}
