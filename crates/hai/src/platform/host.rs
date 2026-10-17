//! The process's channels: one per `[channel.<name>]` section, started together and stopped on one
//! deadline.

use std::{str::FromStr, sync::Arc, time::Duration};

use strum::IntoEnumIterator;
use tokio::{task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{
        channel::{Channel, ChannelAccess},
        intake::Intake,
        runtime::{AgentRuntime, AppContext, registry::AgentRegistry},
    },
    domain::model::Platform,
    error::{AppError, Result},
    platform::{adapter::ChannelAdapter, outbound::OutboundRecorder, telegram::TelegramChannel},
};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(45);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(40);
const _: () = assert!(SHUTDOWN_GRACE.as_secs() < SHUTDOWN_TIMEOUT.as_secs());

/// One channel's runtime, shared by the supervisor that owns it and the loops it starts.
pub(crate) struct ChannelContext {
    pub(crate) adapter: Arc<dyn ChannelAdapter>,
    /// The channel's one inbound path: what it has answered outlives a supervised generation.
    pub(crate) intake: Arc<Intake>,
    pub(crate) agents: AgentRegistry,
    pub(crate) shutdown: CancellationToken,
}

/// A channel the host holds: its runtime and the supervisor restarting its generations.
pub(crate) struct ChannelRuntime {
    pub(crate) context: Arc<ChannelContext>,
    pub(crate) supervisor: JoinHandle<()>,
}

impl ChannelRuntime {
    pub(crate) fn new(context: Arc<ChannelContext>, supervisor: JoinHandle<()>) -> Self {
        Self {
            context,
            supervisor,
        }
    }

    /// Everything every platform shares about running one channel: from an adapter on, they are all
    /// the same, so a platform supplies the adapter and gets back what a supervised run needs.
    pub(crate) fn assemble(
        ctx: &AppContext,
        runtime: &AgentRuntime,
        adapter: Arc<dyn ChannelAdapter>,
        shutdown: CancellationToken,
    ) -> Arc<ChannelContext> {
        let channel: Arc<dyn Channel> = Arc::new(OutboundRecorder::new(
            Arc::clone(&adapter),
            ctx.services.clone(),
            runtime.cfg().model_roles.default.model.clone(),
        ));
        let agents = AgentRegistry::new(channel, runtime.clone(), shutdown.clone());
        let access: Arc<dyn ChannelAccess> = adapter.clone();
        let intake = Arc::new(Intake::new(
            access,
            ctx.services.clone(),
            ctx.settings.clone(),
            ctx.cfg.agent.language,
            agents.clone(),
        ));
        Arc::new(ChannelContext {
            adapter,
            intake,
            agents,
            shutdown,
        })
    }
}

pub struct ChannelHost {
    channels: Vec<ChannelRuntime>,
}

impl ChannelHost {
    pub async fn spawn_all(ctx: &AppContext, runtime: &AgentRuntime) -> Result<Self> {
        let mut host = ChannelHost {
            channels: Vec::new(),
        };
        for (key, section) in &ctx.cfg.channel {
            // Without an explicit `type` the section's own name is the platform.
            let kind = section.kind.clone().unwrap_or_else(|| key.clone());
            let platform = resolve_platform(key, &kind)?;
            // A channel's stop belongs to the channel, not to the platform that starts it.
            let shutdown = CancellationToken::new();
            let channel = match platform {
                Platform::Telegram => {
                    TelegramChannel::spawn(key, section, ctx, runtime, shutdown).await?
                }
            };
            host.channels.push(channel);
        }
        Ok(host)
    }

    pub async fn shutdown(mut self) {
        for channel in &self.channels {
            channel.context.shutdown.cancel();
        }

        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        for channel in std::mem::take(&mut self.channels) {
            let ChannelRuntime {
                context,
                supervisor,
            } = channel;
            let drain = context.agents.drain(SHUTDOWN_GRACE);
            if tokio::time::timeout_at(deadline, async { tokio::join!(supervisor, drain) })
                .await
                .is_err()
            {
                tracing::warn!(channel = %context.adapter.key(), "shutdown timeout, abandoning");
            }
        }
    }
}

/// The platform one `[channel.<name>]` section names; an unknown name fails startup with the names
/// this build supports.
fn resolve_platform(key: &str, kind: &str) -> Result<Platform> {
    Platform::from_str(kind).map_err(|e| {
        AppError::message(format!(
            "channel '{key}': unknown platform '{kind}', supported: {}: {e}",
            Platform::iter()
                .map(|platform| platform.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}

impl Drop for ChannelHost {
    fn drop(&mut self) {
        for channel in &mut self.channels {
            channel.context.shutdown.cancel();
            channel.supervisor.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sections_name_or_its_type_names_the_platform() {
        assert_eq!(
            resolve_platform("telegram", "telegram").expect("a known platform"),
            Platform::Telegram
        );
        assert_eq!(
            resolve_platform("main", "telegram").expect("a known platform"),
            Platform::Telegram
        );
    }

    #[test]
    fn an_unknown_platform_names_the_section_and_what_is_supported() {
        let err = resolve_platform("main", "qq").unwrap_err().to_string();
        assert!(err.contains("channel 'main'"), "{err}");
        assert!(err.contains("unknown platform 'qq'"), "{err}");
        assert!(err.contains("telegram"), "{err}");
    }
}
