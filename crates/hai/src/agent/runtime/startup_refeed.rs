use crate::{agent::runtime::registry::AgentRegistry, domain::service::Services, error::Result};

/// Rings every chat whose accumulation is still unconsumed, once, at generation start. It stops on
/// its own, so it hands back nothing to hold.
pub fn spawn_startup_refeed(services: Services, registry: AgentRegistry) {
    tokio::spawn(async move {
        if let Err(e) = refeed(&services, &registry).await {
            tracing::error!("startup re-feed failed: {e}");
        }
    });
}

async fn refeed(services: &Services, registry: &AgentRegistry) -> Result<()> {
    let chats = services.conversation.chats_with_pending().await?;
    if chats.is_empty() {
        return Ok(());
    }

    tracing::info!(chats = chats.len(), "re-feeding unconsumed changes");
    for chat_id in chats {
        registry.get_or_create(chat_id).await.notify();
    }
    Ok(())
}
