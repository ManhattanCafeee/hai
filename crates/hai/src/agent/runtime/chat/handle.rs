use tokio::sync::{mpsc, oneshot};

use super::scheduler::SchedulerStatus;
use crate::{
    agent::runtime::event::Doorbell,
    agentcore::{Language, PerCatalog},
    domain::vo::ChatId,
};

pub(crate) struct ChatAgentStatus {
    pub scheduler: SchedulerStatus,
    pub step_count: usize,
    pub context_tokens: u32,
    pub conversation_msgs: usize,
    pub turn_elapsed_secs: Option<f64>,
    pub model: String,
}

impl ChatAgentStatus {
    fn render(&self, language: Language) -> String {
        let labels = LABELS.get(language.text_catalog());
        let sched = &self.scheduler;
        let mut lines = vec![format!("{}  {}", labels.model, self.model)];

        let runs_line = if let Some(secs) = self.turn_elapsed_secs {
            format!(
                "{}  {} · {} {:.1}s",
                labels.steps, self.step_count, labels.active, secs
            )
        } else {
            format!("{}  {}", labels.steps, self.step_count)
        };
        lines.push(runs_line);

        if self.context_tokens > 0 {
            lines.push(format!(
                "{}  {}",
                labels.tokens,
                fmt_tokens(self.context_tokens)
            ));
        }

        lines.push(format!(
            "{}  {} {}",
            labels.conversation, self.conversation_msgs, labels.messages
        ));

        let heat = format!(
            "{}  {:.2} / {:.2}",
            labels.heat, sched.heat_value, sched.heat_base
        );
        let heat_line = if sched.window_active {
            let secs = sched.window_closes_in_secs.unwrap_or(0.0) as i64;
            format!("{heat} · {} {}s", labels.window, secs)
        } else {
            heat
        };
        lines.push(heat_line);

        lines.join("\n")
    }
}

struct StatusLabels {
    model: &'static str,
    steps: &'static str,
    active: &'static str,
    tokens: &'static str,
    conversation: &'static str,
    messages: &'static str,
    heat: &'static str,
    window: &'static str,
}

const LABELS: PerCatalog<StatusLabels> = PerCatalog {
    zh: StatusLabels {
        model: "模型",
        steps: "步数",
        active: "进行中",
        tokens: "上下文",
        conversation: "对话",
        messages: "条",
        heat: "热度",
        window: "留意",
    },
    en: StatusLabels {
        model: "model",
        steps: "steps",
        active: "active",
        tokens: "tokens",
        conversation: "conv",
        messages: "msgs",
        heat: "heat",
        window: "watching",
    },
};

fn fmt_tokens(n: u32) -> String {
    if n >= 1000 {
        format!("{}.{}k", n / 1000, (n % 1000) / 100)
    } else {
        n.to_string()
    }
}

#[derive(Clone)]
pub struct ChatAgentRef {
    chat_id: ChatId,
    doorbell: Doorbell,
    status_tx: mpsc::UnboundedSender<oneshot::Sender<ChatAgentStatus>>,
}

impl ChatAgentRef {
    pub(crate) fn new(
        chat_id: ChatId,
        doorbell: Doorbell,
        status_tx: mpsc::UnboundedSender<oneshot::Sender<ChatAgentStatus>>,
    ) -> Self {
        Self {
            chat_id,
            doorbell,
            status_tx,
        }
    }

    /// Rings the chat's doorbell; the changes themselves are already in the database.
    pub fn notify(&self) {
        self.doorbell.notify();
    }

    pub async fn status_text(&self, language: Language) -> Option<String> {
        Some(self.status().await?.render(language))
    }

    async fn status(&self) -> Option<ChatAgentStatus> {
        let (tx, rx) = oneshot::channel();
        if self.status_tx.send(tx).is_err() {
            tracing::warn!(chat_id = %self.chat_id, "Failed to send status query: agent dead");
            return None;
        }
        match rx.await {
            Ok(s) => Some(s),
            Err(_) => {
                tracing::warn!(chat_id = %self.chat_id, "Status query cancelled: agent dropped");
                None
            }
        }
    }
}
