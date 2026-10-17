//! One channel's live ChatAgents: spawned on demand, pruned when their task ends, and drained on
//! shutdown. This is where a chat is bound to the channel it lives on — each agent gets its own
//! handle, plus the runtime and the cancellation token.

use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::{
    sync::{RwLock, mpsc},
    task::JoinHandle,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use super::{
    AgentRuntime,
    chat::{ChatAgent, ChatAgentRef},
    event::Doorbell,
};
use crate::{
    agent::channel::{Channel, ChannelHandle},
    domain::vo::ChatId,
};

#[derive(Clone)]
pub struct AgentRegistry {
    agents: Arc<RwLock<HashMap<ChatId, AgentEntry>>>,
    /// The channel every agent here runs on, unbound: each chat is bound to it as that agent starts.
    channel: Arc<dyn Channel>,
    runtime: AgentRuntime,
    shutdown: CancellationToken,
}

struct AgentEntry {
    handle: ChatAgentRef,
    task: JoinHandle<()>,
}

impl AgentRegistry {
    pub fn new(
        channel: Arc<dyn Channel>,
        runtime: AgentRuntime,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            channel,
            runtime,
            shutdown,
        }
    }

    pub async fn get_or_create(&self, chat_id: ChatId) -> ChatAgentRef {
        if let Some(entry) = self.agents.read().await.get(&chat_id)
            && !entry.task.is_finished()
        {
            return entry.handle.clone();
        }
        let mut agents = self.agents.write().await;
        agents.retain(|_, entry| !entry.task.is_finished());
        if let Some(entry) = agents.get(&chat_id)
            && !entry.task.is_finished()
        {
            return entry.handle.clone();
        }
        let entry = self.spawn(chat_id);
        let handle = entry.handle.clone();
        agents.insert(chat_id, entry);
        handle
    }

    /// A live agent for the chat, without spawning one; a finished or absent entry reads as none.
    pub async fn get(&self, chat_id: ChatId) -> Option<ChatAgentRef> {
        let agents = self.agents.read().await;
        let entry = agents.get(&chat_id)?;
        if entry.task.is_finished() {
            return None;
        }
        Some(entry.handle.clone())
    }

    fn spawn(&self, chat_id: ChatId) -> AgentEntry {
        let doorbell = Doorbell::new();
        let (status_tx, status_rx) = mpsc::unbounded_channel();
        let runtime = self.runtime.clone();
        let chat = ChannelHandle::new(self.channel.clone(), chat_id);
        let shutdown = self.shutdown.clone();
        let agents = self.agents.clone();
        let doorbell_for_turn = doorbell.clone();
        let task = tokio::spawn(async move {
            match ChatAgent::new(runtime, chat, shutdown).await {
                Ok(mut agent) => agent.run(doorbell_for_turn, status_rx).await,
                Err(e) => tracing::error!(%chat_id, "Failed to create agent: {e}"),
            }
            agents.write().await.remove(&chat_id);
        });
        AgentEntry {
            handle: ChatAgentRef::new(chat_id, doorbell, status_tx),
            task,
        }
    }

    pub async fn drain(&self, timeout: Duration) {
        let tasks: Vec<JoinHandle<()>> = {
            let mut agents = self.agents.write().await;
            agents.drain().map(|(_, entry)| entry.task).collect()
        };
        let deadline = Instant::now() + timeout;
        for task in tasks {
            if tokio::time::timeout_at(deadline, task).await.is_err() {
                tracing::warn!("agent drain timed out, remaining agents abandoned");
            }
        }
    }
}
