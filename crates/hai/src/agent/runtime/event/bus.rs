use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

pub use crate::domain::vo::{AgentEvent, AgentEventPayload};
use crate::domain::{repo::EventRepo, vo::ChatId};

const FLUSH_INTERVAL: Duration = Duration::from_millis(200);
const FLUSH_BATCH: usize = 50;

#[derive(Clone)]
pub struct AgentEventBus {
    tx: mpsc::UnboundedSender<AgentEvent>,
    flush_tx: mpsc::UnboundedSender<oneshot::Sender<()>>,
}

impl AgentEventBus {
    pub fn new(event_repo: EventRepo) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (flush_tx, flush_rx) = mpsc::unbounded_channel();
        tokio::spawn(collector(rx, flush_rx, event_repo));
        Self { tx, flush_tx }
    }

    pub fn emit(&self, chat_id: ChatId, payload: AgentEventPayload) {
        let kind = payload.kind();
        if self.tx.send(AgentEvent { chat_id, payload }).is_err() {
            tracing::warn!(kind = %kind, "Agent event dropped: the collector is gone");
        }
    }

    pub async fn flush(&self) {
        const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
        let (ack, ack_rx) = oneshot::channel();
        if self.flush_tx.send(ack).is_err() {
            tracing::warn!("Agent event flush skipped: the collector is gone");
            return;
        }
        if tokio::time::timeout(FLUSH_TIMEOUT, ack_rx).await.is_err() {
            tracing::warn!("Agent event flush timed out");
        }
    }
}

async fn collector(
    mut rx: mpsc::UnboundedReceiver<AgentEvent>,
    mut flush_rx: mpsc::UnboundedReceiver<oneshot::Sender<()>>,
    event_repo: EventRepo,
) {
    let mut batch = Vec::with_capacity(FLUSH_BATCH);
    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Some(event) => {
                    batch.push(event);
                    if batch.len() >= FLUSH_BATCH {
                        flush(&batch, &event_repo).await;
                        batch.clear();
                    }
                }
                // Every sender is gone, so no flush request can arrive either: this is the last
                // chance to write what is held, and the collector ends after it.
                None => {
                    flush(&batch, &event_repo).await;
                    break;
                }
            },
            Some(ack) = flush_rx.recv() => {
                while let Ok(event) = rx.try_recv() {
                    batch.push(event);
                    if batch.len() >= FLUSH_BATCH {
                        flush(&batch, &event_repo).await;
                        batch.clear();
                    }
                }
                if !batch.is_empty() {
                    flush(&batch, &event_repo).await;
                    batch.clear();
                }
                let _ = ack.send(());
            }
            _ = tokio::time::sleep(FLUSH_INTERVAL) => {
                if !batch.is_empty() {
                    flush(&batch, &event_repo).await;
                    batch.clear();
                }
            }
        }
    }
}

async fn flush(batch: &[AgentEvent], event_repo: &EventRepo) {
    for event in batch {
        if let Err(e) = event_repo.insert(event.to_json_value()).await {
            tracing::warn!(
                kind = %event.payload.kind(),
                error = %e,
                "Failed to flush agent event"
            );
        }
    }
}
