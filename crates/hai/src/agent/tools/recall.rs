use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    agent::{
        context::{
            fmt::format_time_dyn,
            sections::{memory::memory_element, topic::topic_element},
        },
        runtime::turn::ToolContext,
        tools::util::bounded_limit,
    },
    agentcore::{
        render::{elements::Node, render_json},
        tool::{AgentTool, ToolError, tool_data},
    },
    domain::{
        service::{RecallItem, RecallQuery, RecallScope, ScorerErrorPolicy, Services, recall},
        vo::{ChatId, TimeRange},
    },
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecallScopeArg {
    All,
    Memory,
    Topic,
}

impl From<RecallScopeArg> for RecallScope {
    fn from(arg: RecallScopeArg) -> Self {
        match arg {
            RecallScopeArg::All => Self::All,
            RecallScopeArg::Memory => Self::Memory,
            RecallScopeArg::Topic => Self::Topic,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RecallArgs {
    /// A semantic question ("whatever became of the plan we discussed last time")
    pub query: Option<String>,
    /// Terms to match exactly: names, verbatim error text, proper nouns
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    pub scope: Option<RecallScopeArg>,
    /// Start of the range (ISO 8601)
    #[schemars(with = "Option<String>")]
    pub since: Option<jiff::Timestamp>,
    /// End of the range (ISO 8601)
    #[schemars(with = "Option<String>")]
    pub until: Option<jiff::Timestamp>,
    pub limit: Option<u32>,
}

/// Recalls past matters: searches this chat's personal memories and topic archives. Reach for it when the user mentions a name, date or earlier agreement you do not know; when they ask about "before / last time / what we said"; or when you are about to record a new memory and want to check whether one already exists. Give `query` a semantic question and `keywords` the terms to match exactly (names, verbatim error text, proper nouns); both may be given at once. With only since/until it lists the range newest first. Results carry summaries only; for the full text use read topic://<id> or read memory://<id>.
#[hai_macros::tool]
pub struct Recall {
    pub chat_id: ChatId,
    pub services: Services,
}

impl Recall {
    async fn exec(&self, args: RecallArgs) -> Result<Value, ToolError> {
        let limit = i64::from(bounded_limit(args.limit, 8));

        let query = RecallQuery {
            query: args.query.filter(|text| !text.trim().is_empty()),
            keywords: args.keywords.unwrap_or_default(),
            scope: args.scope.unwrap_or(RecallScopeArg::All).into(),
            range: TimeRange::new(args.since, args.until),
            limit,
            on_error: ScorerErrorPolicy::Fail,
        };

        let hits = recall(&self.services, self.chat_id, query).await?;
        if hits.is_empty() {
            return tool_data(serde_json::json!({
                "recall": "Nothing matching was found.",
                "count": 0,
            }));
        }

        let count = hits.len();
        let section = Node::tag("recall")
            .attr("as_of", format_time_dyn(jiff::Timestamp::now()))
            .children(hits.into_iter().map(hit_element).collect());
        tool_data(serde_json::json!({
            "recall": render_json(&section),
            "count": count,
        }))
    }
}

fn hit_element(item: RecallItem) -> Node {
    match item {
        RecallItem::Memory(memory) => memory_element(&memory),
        RecallItem::Topic(topic) => topic_element(&topic, false),
    }
}

pub fn tools(ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    vec![Arc::new(Recall {
        chat_id: ctx.chat.chat_id(),
        services: ctx.services.clone(),
    })]
}
