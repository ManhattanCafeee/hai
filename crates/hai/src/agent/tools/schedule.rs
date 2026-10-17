use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    agent::{channel::ChannelHandle, runtime::turn::ToolContext, tools::util::required},
    agentcore::tool::{AgentTool, ToolError, tool_data, tool_err, tool_ok},
    domain::{
        service::ScheduledTaskService,
        vo::{RepeatInterval, ScheduledTaskId},
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
pub enum ScheduleOp {
    Create,
    List,
    Cancel,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleArgs {
    pub op: ScheduleOp,
    /// Absolute instant in ISO 8601, e.g. 2026-09-01T09:00:00Z (required for `create`)
    #[schemars(with = "Option<String>")]
    pub at: Option<jiff::Timestamp>,
    /// Optional for `create`
    #[schemars(with = "Option<i64>")]
    pub every_secs: Option<RepeatInterval>,
    /// What to do when it fires (required for `create`)
    pub description: Option<String>,
    /// Used by `list`
    pub include_inactive: Option<bool>,
    /// Required for `cancel`
    #[schemars(with = "Option<Uuid>")]
    pub task_id: Option<ScheduledTaskId>,
}

/// Scheduled task maintenance. Ops: create (schedule a wake-up; needs at/description) / list (list this conversation's tasks) / cancel (cancel one; needs task_id).
#[hai_macros::tool]
pub struct Schedule {
    pub chat: ChannelHandle,
    pub tasks: ScheduledTaskService,
}

impl Schedule {
    async fn exec(&self, args: ScheduleArgs) -> Result<Value, ToolError> {
        let op: &str = args.op.into();
        let bot_id = self.chat.key().clone();
        let chat_id = self.chat.chat_id();
        match args.op {
            ScheduleOp::Create => {
                let fire_at = required(args.at, "at", op)?;
                let description = required(args.description, "description", op)?;
                if fire_at <= jiff::Timestamp::now() {
                    return Err(tool_err("`at` must be in the future"));
                }
                let task = self
                    .tasks
                    .create(&bot_id, chat_id, &description, fire_at, args.every_secs)
                    .await?;
                tool_data(json!({
                    "task_id": task.id,
                    "fire_at": task.fire_at.to_string(),
                }))
            }
            ScheduleOp::List => {
                let rows = if args.include_inactive.unwrap_or(false) {
                    self.tasks.list_all(&bot_id, chat_id).await?
                } else {
                    self.tasks.list_active(&bot_id, chat_id).await?
                };
                let items: Vec<_> = rows
                    .into_iter()
                    .map(|t| {
                        json!({
                            "task_id": t.id,
                            "description": t.description,
                            "fire_at": t.fire_at.to_string(),
                            "every_secs": t.every_secs,
                            "is_active": t.is_active,
                        })
                    })
                    .collect();
                tool_data(json!({ "tasks": items }))
            }
            ScheduleOp::Cancel => {
                let task_id = required(args.task_id, "task_id", op)?;
                self.tasks.cancel(&bot_id, chat_id, task_id).await?;
                tool_ok()
            }
        }
    }
}

pub fn tools(ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    vec![Arc::new(Schedule {
        chat: ctx.chat.clone(),
        tasks: ctx.services.scheduled_task.clone(),
    })]
}
