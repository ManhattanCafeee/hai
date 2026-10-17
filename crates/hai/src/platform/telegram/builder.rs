use std::{sync::Arc, time::Duration};

use teloxide::{Bot as TgBot, dispatching::ShutdownToken};
use tokio::{sync::oneshot, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use super::{
    adapter::TelegramAdapter, config::TelegramConfig, dispatcher::TelegramDispatcher, menu,
};
use crate::{
    agent::runtime::{
        AgentRuntime, AppContext, startup_refeed::spawn_startup_refeed,
        task_watcher::spawn_task_watcher,
    },
    config::schema::ChannelSection,
    domain::{model::Platform, service::Services, vo::ChannelKey},
    error::Result,
    platform::{
        adapter::ChannelAdapter,
        host::{ChannelContext, ChannelRuntime},
        task::TaskGuard,
    },
};

/// The Telegram side of one configured channel: its adapter, its own settings, and the supervision around
/// them.
pub struct TelegramChannel;

impl TelegramChannel {
    pub async fn spawn(
        key: &str,
        section: &ChannelSection,
        ctx: &AppContext,
        runtime: &AgentRuntime,
        shutdown: CancellationToken,
    ) -> Result<ChannelRuntime> {
        let cfg = TelegramConfig::parse(key, &section.settings)?;
        let client = TgBot::new(&cfg.bot_token);
        let channel_key = ChannelKey::new(Platform::Telegram.as_str(), key);
        let account = ctx
            .services
            .platform
            .ensure_bot_account(Platform::Telegram, &channel_key)
            .await?;
        let adapter: Arc<dyn ChannelAdapter> =
            Arc::new(TelegramAdapter::new(client.clone(), channel_key, cfg, account.id).await?);
        let context = ChannelRuntime::assemble(ctx, runtime, adapter, shutdown);
        // Once per channel, not per generation: the menu is the command table's, and it does not change
        // while the process runs.
        menu::install(&client).await;
        let supervisor = tokio::spawn(supervise(
            Arc::clone(&context),
            client,
            ctx.services.clone(),
        ));
        Ok(ChannelRuntime::new(context, supervisor))
    }
}

const HEALTHY_GENERATION: Duration = Duration::from_secs(60);
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(60);
const DISPATCHER_STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// Which supervised task ended on its own.
enum Subsystem {
    Dispatcher,
    Watcher,
}

impl Subsystem {
    fn label(&self) -> &'static str {
        match self {
            Self::Dispatcher => "dispatcher",
            Self::Watcher => "scheduled task watcher",
        }
    }
}

enum GenerationExit {
    Cancelled,
    Died(Subsystem),
}

/// Reports a supervised task that ended without the shutdown token being cancelled: an unexpected
/// exit is a warning, a panic an error.
fn note_task_end(
    label: impl std::fmt::Display,
    subsystem: Subsystem,
    result: Result<(), tokio::task::JoinError>,
) {
    match result {
        Ok(()) => tracing::warn!(channel = %label, "{} exited unexpectedly", subsystem.label()),
        Err(e) if e.is_panic() => {
            tracing::error!(channel = %label, "{} panicked", subsystem.label())
        }
        Err(_) => {}
    }
}

/// Graceful before forceful: `ShutdownToken::shutdown` lets teloxide drop the
/// in-flight GetUpdates and stop polling, whereas aborting a live dispatcher
/// turns that cancellation into a spurious ERROR. A dispatcher that ignored the
/// token for `DISPATCHER_STOP_TIMEOUT` is aborted.
async fn stop_dispatcher(shutdown: Option<ShutdownToken>, task: &mut JoinHandle<()>) {
    if let Some(token) = &shutdown {
        let _ = token.shutdown();
    }
    if tokio::time::timeout(DISPATCHER_STOP_TIMEOUT, &mut *task)
        .await
        .is_err()
    {
        task.abort();
        let _ = task.await;
    }
}

async fn run_generation(
    context: &ChannelContext,
    client: &TgBot,
    services: &Services,
) -> GenerationExit {
    let label = context.adapter.key();
    let dispatcher = TelegramDispatcher::new(client.clone(), Arc::clone(&context.intake)).await;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let mut d_task = TaskGuard::spawn(dispatcher.run(shutdown_tx));
    tracing::info!(channel = %label, "dispatcher started");
    let mut w_task = TaskGuard::wrap(spawn_task_watcher(
        services.clone(),
        context.agents.clone(),
        context.adapter.key().clone(),
        context.shutdown.clone(),
    ));
    // One shot per generation: refeeding only rings doorbells, so there is nothing to await or stop.
    spawn_startup_refeed(services.clone(), context.agents.clone());
    let shutdown = shutdown_rx.await.ok();

    tokio::select! {
        _ = context.shutdown.cancelled() => {
            stop_dispatcher(shutdown, d_task.handle()).await;
            w_task.join().await;
            tracing::info!(channel = %label, "supervisor stopped");
            GenerationExit::Cancelled
        }
        d_done = d_task.handle() => {
            // The watcher is not stopped here: leaving this generation drops its guard, which aborts
            // it, so no restart can end up with two of them.
            if context.shutdown.is_cancelled() {
                tracing::info!(channel = %label, "supervisor stopped");
                return GenerationExit::Cancelled;
            }
            note_task_end(label, Subsystem::Dispatcher, d_done);
            GenerationExit::Died(Subsystem::Dispatcher)
        }
        w_done = w_task.handle() => {
            let stopped = if context.shutdown.is_cancelled() {
                GenerationExit::Cancelled
            } else {
                note_task_end(label, Subsystem::Watcher, w_done);
                GenerationExit::Died(Subsystem::Watcher)
            };
            stop_dispatcher(shutdown, d_task.handle()).await;
            if matches!(stopped, GenerationExit::Cancelled) {
                tracing::info!(channel = %label, "supervisor stopped");
            }
            stopped
        }
    }
}

async fn supervise(context: Arc<ChannelContext>, client: TgBot, services: Services) {
    let label = context.adapter.key().to_string();
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = tokio::time::Instant::now();
        let exit = run_generation(&context, &client, &services).await;
        if let GenerationExit::Died(subsystem) = exit {
            if started.elapsed() >= HEALTHY_GENERATION {
                backoff = Duration::from_secs(1);
            }
            tracing::info!(
                channel = %label,
                subsystem = subsystem.label(),
                backoff = backoff.as_secs(),
                "restarting channel tasks"
            );
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(RESTART_BACKOFF_MAX);
        } else {
            return;
        }
    }
}
