use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, IntoStaticStr};
use uuid::Uuid;

use super::{AgentCommand, MessageId, ScheduledTaskId};

/// One change to a chat, in the order it happened. The content is not copied here: the message row
/// holds the current value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatEntry {
    pub id: Uuid,
    /// When the change happened: a created message carries the platform's send time, an edited one
    /// its edit time, a command or a scheduled task the moment it was enqueued.
    pub at: jiff::Timestamp,
    pub audience: EntryAudience,
    pub kind: EntryKind,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Display,
    EnumIter,
    EnumString,
    IntoStaticStr,
    Serialize,
    Deserialize,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EntryAudience {
    Agent,
    Bystanders,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EntryKind {
    MessageCreated {
        message_id: MessageId,
    },
    MessageEdited {
        message_id: MessageId,
        edited_at: jiff::Timestamp,
    },
    Command(AgentCommand),
    Scheduled {
        task_id: Option<ScheduledTaskId>,
        description: String,
    },
}

impl ChatEntry {
    pub fn new(audience: EntryAudience, kind: EntryKind, at: jiff::Timestamp) -> Self {
        Self {
            id: Uuid::now_v7(),
            at,
            audience,
            kind,
        }
    }

    pub fn message_id(&self) -> Option<MessageId> {
        match &self.kind {
            EntryKind::MessageCreated { message_id }
            | EntryKind::MessageEdited { message_id, .. } => Some(*message_id),
            EntryKind::Command(_) | EntryKind::Scheduled { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_round_trips_through_its_wire_form() {
        let at = jiff::Timestamp::from_second(1_700_000_000).expect("timestamp");
        for kind in [
            EntryKind::MessageCreated {
                message_id: MessageId(7),
            },
            EntryKind::MessageEdited {
                message_id: MessageId(7),
                edited_at: at,
            },
            EntryKind::Command(AgentCommand::Digest(3)),
            EntryKind::Scheduled {
                task_id: Some(ScheduledTaskId(Uuid::from_u128(9))),
                description: "take the medication".into(),
            },
        ] {
            let entry = ChatEntry::new(EntryAudience::Agent, kind.clone(), at);
            let wire = serde_json::to_value(&entry).expect("serializable");
            let back: ChatEntry = serde_json::from_value(wire).expect("deserializable");
            assert_eq!(back, entry);
            assert_eq!(back.kind, kind);
        }
    }
}
