pub mod account;
pub mod attachment;
pub mod chat;
pub mod chat_preference;
pub mod conversation_record;
pub mod event;
pub mod identity;
pub mod knowledge_chunk;
pub mod knowledge_document;
pub mod memory;
pub mod message;
pub mod perception;
pub mod scheduled_task;
pub mod topic;

pub use account::{Account, Platform};
pub use attachment::{Attachment, NewAttachment};
pub use chat::{Chat, ChatType, NewChat};
pub use chat_preference::ChatPreference;
pub use conversation_record::ConversationRecord;
pub use event::Event;
pub use identity::Identity;
pub use knowledge_chunk::KnowledgeChunk;
pub use knowledge_document::{KnowledgeDocument, KnowledgeDocumentMeta};
pub use memory::{Memory, MemoryKind, MemoryStatus, NewMemory};
pub use message::{Message, MessageRole, NewMessage};
pub use perception::Perception;
pub use scheduled_task::ScheduledTask;
use sqlx::ValueRef;
pub(crate) use topic::embed_text;
pub use topic::{Topic, TopicNote, TopicStatus};

/// NULL bridge for nullable timestamp columns: `jiff-sqlx` implements `Decode` / `Type` only
/// for its own wrapper, and `FromRow`'s `try_from` does not turn a NULL into an absent `Option`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SqlxNullableTimestamp(pub(crate) Option<jiff_sqlx::Timestamp>);

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for SqlxNullableTimestamp {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        if value.is_null() {
            Ok(SqlxNullableTimestamp(None))
        } else {
            <jiff_sqlx::Timestamp as sqlx::Decode<'r, sqlx::Postgres>>::decode(value)
                .map(|t| SqlxNullableTimestamp(Some(t)))
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for SqlxNullableTimestamp {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <jiff_sqlx::Timestamp as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl From<SqlxNullableTimestamp> for Option<jiff::Timestamp> {
    fn from(v: SqlxNullableTimestamp) -> Self {
        v.0.map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;
    use crate::{
        domain::vo::{
            AgentEventPayload, AttachmentKind, AttachmentParser, BatchComposition, ChatEntry,
            EntryAudience, EntryKind, EventKind, MessageId, ModelRetryReason, StartedBy,
            StepNumber, StepOutput, TurnEndReason, TurnNumber, WakeReasonKind,
        },
        test_support::assert_names_agree,
    };

    /// The count the audit trail shows is the one attention read.
    #[test]
    fn a_batch_composition_counts_the_audiences() {
        let at = jiff::Timestamp::from_second(1_700_000_000).expect("timestamp");
        let entry = |audience| {
            ChatEntry::new(
                audience,
                EntryKind::MessageCreated {
                    message_id: MessageId(1),
                },
                at,
            )
        };

        let counts = BatchComposition::of(&[
            entry(EntryAudience::Agent),
            entry(EntryAudience::Bystanders),
            entry(EntryAudience::Bystanders),
        ]);

        assert_eq!((counts.addressed, counts.bystanders), (1, 2));
    }

    /// A turn record written before `started_by` and `composition` existed must still decode: the
    /// audit trail is read back by `hai log`, and a field a record never carried is not corruption.
    /// The fixture spells the message count the way those records did, which is what the field's alias
    /// keeps readable.
    #[test]
    fn a_turn_record_from_before_the_ground_fields_still_decodes() {
        let old = serde_json::json!({
            "event": "turn-started",
            "turn": 3,
            "reason": "Observe",
            "msg_count": 17,
            "full_prompt": ""
        });

        match serde_json::from_value::<AgentEventPayload>(old).expect("a pre-field record decodes")
        {
            AgentEventPayload::TurnStarted {
                started_by,
                composition,
                ..
            } => {
                assert_eq!(started_by, None);
                assert_eq!(composition, None);
            }
            other => panic!("expected a turn record; got {other:?}"),
        }
    }

    #[test]
    fn domain_enum_names_agree_between_serde_and_strum() {
        assert_names_agree(MemoryKind::iter(), |kind| kind.as_str());
        assert_names_agree(MemoryStatus::iter(), |status| status.as_str());
        assert_names_agree(EntryAudience::iter(), |audience| audience.into());
        assert_names_agree(StartedBy::iter(), |started_by| started_by.into());
        assert_names_agree(TopicStatus::iter(), |status| status.as_str());
        assert_names_agree(Platform::iter(), |platform| platform.into());
        assert_names_agree(ChatType::iter(), |chat_type| chat_type.into());
        assert_names_agree(AttachmentKind::iter(), |kind| kind.name());
        assert_names_agree(AttachmentParser::iter(), |parser| parser.name());
        assert_names_agree(ModelRetryReason::iter(), |reason| reason.into());
        assert_names_agree(EventKind::iter(), |kind| kind.as_str());
        assert_names_agree(WakeReasonKind::iter(), |kind| kind.as_str());
        assert_names_agree(
            [
                AgentEventPayload::TurnStarted {
                    turn: TurnNumber(0),
                    reason: WakeReasonKind::Observe,
                    msg_count: 0,
                    full_prompt: String::new(),
                    started_by: None,
                    composition: None,
                },
                AgentEventPayload::ToolCall {
                    turn: TurnNumber(0),
                    step: StepNumber(0),
                    tool: String::new(),
                    args: String::new(),
                },
                AgentEventPayload::ToolCallResult {
                    turn: TurnNumber(0),
                    step: StepNumber(0),
                    tool: String::new(),
                    summary: String::new(),
                    success: false,
                },
                AgentEventPayload::StepCompleted {
                    turn: TurnNumber(0),
                    step: StepNumber(0),
                    output: StepOutput {
                        turn: TurnNumber(0),
                        step: StepNumber(0),
                        reasoning: None,
                        response: String::new(),
                    },
                },
                AgentEventPayload::TurnEnded {
                    turn: TurnNumber(0),
                    reason: TurnEndReason::Failed {
                        error: String::new(),
                    },
                },
                AgentEventPayload::ModelRetry {
                    turn: TurnNumber(0),
                    reason: ModelRetryReason::TimeoutRetry,
                },
                AgentEventPayload::WrapUpStarted,
                AgentEventPayload::WrapUpCompleted {
                    step_count: 0,
                    context_tokens: 0,
                    summary: String::new(),
                },
                AgentEventPayload::WrapUpFailed {
                    error: String::new(),
                },
            ],
            |event| event.kind().as_str(),
        );
    }
}
