use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{
    agent::runtime::registry::AgentRegistry,
    domain::{
        service::Services,
        vo::{ChannelKey, ChatEntry, EntryAudience, EntryKind},
    },
    error::Result,
};

pub fn spawn_task_watcher(
    services: Services,
    registry: AgentRegistry,
    key: ChannelKey,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!(channel = %key, "scheduled task watcher started");
        if let Err(e) = run(&services, &registry, &key, &shutdown).await {
            tracing::error!(channel = %key, "scheduled task watcher stopped: {e}");
        }
    })
}

async fn run(
    services: &Services,
    registry: &AgentRegistry,
    key: &ChannelKey,
    shutdown: &CancellationToken,
) -> Result<()> {
    loop {
        let now = jiff::Timestamp::now();
        let due = services.scheduled_task.due(key, now).await?;

        for task in due {
            let chat_id = task.chat_id;
            let entry = ChatEntry::new(
                EntryAudience::Agent,
                EntryKind::Scheduled {
                    task_id: Some(task.id),
                    description: task.description.clone(),
                },
                now,
            );
            services.scheduled_task.fire(&task, &entry, now).await?;
            registry.get_or_create(chat_id).await.notify();
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            _ = shutdown.cancelled() => {
                tracing::info!(channel = %key, "scheduled task watcher stopped");
                return Ok(());
            }
        }
    }
}
