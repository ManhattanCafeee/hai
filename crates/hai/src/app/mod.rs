//! The composition root: logging, the process context, the agent runtime, channel startup and
//! shutdown orchestration.

use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use crate::{
    agent::runtime::{AgentRuntime, AppContext},
    config::AppConfig,
    error::Result,
    platform::host::ChannelHost,
};

pub async fn serve(cfg: Arc<AppConfig>) -> Result<()> {
    let level = cfg.logging.level()?;
    let filter = EnvFilter::new(format!("{},rmcp=warn,html5ever=error", level));
    tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(filter)
        .init();

    let ctx = AppContext::new(cfg).await?;
    let runtime = AgentRuntime::new(ctx.clone()).await?;
    let host = ChannelHost::spawn_all(&ctx, &runtime).await?;

    wait_for_shutdown_signal().await;
    tracing::info!("Shutting down...");
    host.shutdown().await;
    runtime.event_bus().flush().await;
    tracing::info!("Shutdown complete");
    Ok(())
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
    }
}
