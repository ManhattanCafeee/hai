use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    agent::{
        runtime::turn::ToolContext,
        tools::util::{deserialize_option_lenient_i64_vec, required},
    },
    agentcore::tool::{AgentTool, ToolError, tool_data, tool_err, tool_ok},
    domain::{
        service::TopicService,
        vo::{ChatId, MessageId, TopicId},
    },
};

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
pub enum TopicOp {
    Open,
    Note,
    Revise,
    Close,
    Link,
    Drop,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TopicArgs {
    pub op: TopicOp,
    /// Required for every op except `open`
    #[schemars(with = "Option<Uuid>")]
    pub topic_id: Option<TopicId>,
    /// Required for `open`; `revise` overwrites it
    pub title: Option<String>,
    /// Required for `open`; `revise` overwrites it
    pub summary: Option<String>,
    /// Required for `note`
    pub text: Option<String>,
    /// Used by `open`, `note` and `link`
    #[serde(default, deserialize_with = "deserialize_option_lenient_i64_vec")]
    #[schemars(with = "Option<Vec<i64>>")]
    pub message_ids: Option<Vec<MessageId>>,
}

/// Topic archive maintenance: the record of one matter over a stretch of time. Ops: open (start an archive; needs title/summary) / note (append raw material; needs topic_id/text) / revise (change title or summary, at least one) / close (wrap it up; needs topic_id) / link (attach messages; needs topic_id/message_ids) / drop (delete the whole archive; needs topic_id). The summary is the retrieval surface — what this is about at a glance, kept short — and the details go into notes; notes are append-only and never overwritten, so revising the summary loses nothing.
#[hai_macros::tool]
pub struct Topic {
    pub chat_id: ChatId,
    pub topics: TopicService,
}

impl Topic {
    async fn exec(&self, args: TopicArgs) -> Result<Value, ToolError> {
        let op: &str = args.op.into();
        let ids: Vec<MessageId> = args.message_ids.unwrap_or_default();
        match args.op {
            TopicOp::Open => {
                let title = required(args.title, "title", op)?;
                let summary = required(args.summary, "summary", op)?;
                let topic = self
                    .topics
                    .open(self.chat_id, &title, &summary, &ids)
                    .await?;
                tool_data(serde_json::json!({ "topic_id": topic.id.0 }))
            }
            TopicOp::Note => {
                let topic_id = required(args.topic_id, "topic_id", op)?;
                let text = required(args.text, "text", op)?;
                self.topics
                    .note(self.chat_id, topic_id, &text, &ids)
                    .await?;
                tool_ok()
            }
            TopicOp::Revise => {
                let topic_id = required(args.topic_id, "topic_id", op)?;
                if args.title.is_none() && args.summary.is_none() {
                    return Err(tool_err("revise requires title or summary"));
                }
                self.topics
                    .revise(
                        self.chat_id,
                        topic_id,
                        args.title.as_deref(),
                        args.summary.as_deref(),
                    )
                    .await?;
                tool_ok()
            }
            TopicOp::Close => {
                let topic_id = required(args.topic_id, "topic_id", op)?;
                self.topics.close(self.chat_id, topic_id).await?;
                tool_ok()
            }
            TopicOp::Link => {
                let topic_id = required(args.topic_id, "topic_id", op)?;
                if ids.is_empty() {
                    return Err(tool_err("link requires message_ids"));
                }
                self.topics.link(self.chat_id, topic_id, &ids).await?;
                tool_ok()
            }
            TopicOp::Drop => {
                let topic_id = required(args.topic_id, "topic_id", op)?;
                self.topics.drop(self.chat_id, topic_id).await?;
                tool_ok()
            }
        }
    }
}

pub fn tools(ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    vec![Arc::new(Topic {
        chat_id: ctx.chat.chat_id(),
        topics: ctx.services.topic.clone(),
    })]
}
