use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    agent::{
        context::sections::memory::memory_element,
        runtime::turn::ToolContext,
        tools::util::{deserialize_option_lenient_i64_vec, required},
    },
    agentcore::{
        render::{elements::Node, render_json},
        tool::{AgentTool, ToolError, tool_data, tool_err, tool_ok},
    },
    domain::{
        model::MemoryKind,
        service::MemoryService,
        vo::{AccountId, ChatId, MemoryId, MessageId},
    },
};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordMemoryCategory {
    UserFact,
    Knowledge,
    Note,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    JsonSchema,
    strum::IntoStaticStr,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MemoryOp {
    Record,
    Revise,
    Invalidate,
    Forget,
    Pin,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryArgs {
    pub op: MemoryOp,
    /// Required for every op except `record`
    #[schemars(with = "Option<Uuid>")]
    pub id: Option<MemoryId>,
    /// Required for `record`; `revise` overwrites it
    pub content: Option<String>,
    /// Required for `record`
    pub category: Option<RecordMemoryCategory>,
    /// Required when `category` is `user_fact`
    #[schemars(with = "Option<i64>")]
    pub account_id: Option<AccountId>,
    /// Required for `pin`
    pub pinned: Option<bool>,
    /// For `record`: which messages this memory came from
    #[serde(default, deserialize_with = "deserialize_option_lenient_i64_vec")]
    #[schemars(with = "Option<Vec<i64>>")]
    pub message_ids: Option<Vec<MessageId>>,
}

/// Memory maintenance, one independent fact per entry. Ops: record (write one; needs category/content, plus account_id for user_fact) / revise (change the content; needs id/content) / invalidate (needs id; the row stays for traceability, but recall stops returning it) / forget (delete for good; needs id) / pin (mark as long-lived; needs id/pinned; pinned entries stay resident at the head of the section and are limited in number). record returns the memory id and similar entries (on a duplicate, the returned id is the existing row's); if one of them is about the same fact, use revise/invalidate instead of recording another.
#[hai_macros::tool]
pub struct Memory {
    pub chat_id: ChatId,
    pub memories: MemoryService,
}

impl Memory {
    async fn exec(&self, args: MemoryArgs) -> Result<Value, ToolError> {
        let op: &str = args.op.into();
        match args.op {
            MemoryOp::Record => {
                let category = required(args.category, "category", op)?;
                let content = required(args.content, "content", op)?;
                let kind = match category {
                    RecordMemoryCategory::UserFact => {
                        if args.account_id.is_none() {
                            return Err(tool_err("account_id is required for 'user_fact'"));
                        }
                        MemoryKind::UserFact
                    }
                    RecordMemoryCategory::Knowledge => MemoryKind::Knowledge,
                    RecordMemoryCategory::Note => MemoryKind::Note,
                };
                let source: Vec<MessageId> = args.message_ids.unwrap_or_default();
                let outcome = self
                    .memories
                    .record(kind, self.chat_id, content, args.account_id, &source)
                    .await?;
                tool_data(serde_json::json!({
                    "id": outcome.memory.id.0,
                    "duplicate": outcome.duplicate,
                    "similar": render_json(&similar_section(&outcome.similar)),
                }))
            }
            MemoryOp::Revise => {
                let id = required(args.id, "id", op)?;
                let content = required(args.content, "content", op)?;
                self.memories.revise(self.chat_id, id, content).await?;
                tool_ok()
            }
            MemoryOp::Invalidate => {
                let id = required(args.id, "id", op)?;
                self.memories.invalidate(self.chat_id, id).await?;
                tool_ok()
            }
            MemoryOp::Forget => {
                let id = required(args.id, "id", op)?;
                self.memories.forget(self.chat_id, id).await?;
                tool_ok()
            }
            MemoryOp::Pin => {
                let id = required(args.id, "id", op)?;
                let pinned = required(args.pinned, "pinned", op)?;
                self.memories.pin(self.chat_id, id, pinned).await?;
                tool_ok()
            }
        }
    }
}

fn similar_section(memories: &[crate::domain::model::Memory]) -> Node {
    Node::tag("similar").children(memories.iter().map(memory_element).collect())
}

pub fn tools(ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    vec![Arc::new(Memory {
        chat_id: ctx.chat.chat_id(),
        memories: ctx.services.memory.clone(),
    })]
}
