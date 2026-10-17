use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};

use super::{
    super::event::{Doorbell, WakeEvents},
    ChatAgent, ChatAgentState, RETRY_BACKOFF,
    handle::ChatAgentStatus,
    scheduler::Decision,
};
use crate::{
    agent::runtime::types::BusySignal,
    domain::vo::{AgentEventPayload, StartedBy},
    error::Result,
};

enum NextStep {
    Activate(StartedBy),
    WrapUp,
    Idle,
    Exit,
}

impl ChatAgent {
    pub(crate) async fn run(
        &mut self,
        doorbell: Doorbell,
        mut status_rx: mpsc::UnboundedReceiver<oneshot::Sender<ChatAgentStatus>>,
    ) {
        let shutdown = self.shutdown.clone();
        loop {
            match self.state.take() {
                ChatAgentState::Idle => {
                    tokio::select! {
                        biased;
                        _ = shutdown.cancelled() => break,
                        next = self.idle_tick(&doorbell, &mut status_rx) => match next {
                            NextStep::Activate(started_by) => {
                                self.dispatch(&doorbell, started_by).await
                            }
                            NextStep::Idle => self.state = ChatAgentState::Idle,
                            NextStep::WrapUp => self.start_wrap_up(&doorbell).await,
                            NextStep::Exit => break,
                        },
                    }
                }
                ChatAgentState::Busy {
                    mut result_rx,
                    started_at,
                } => {
                    let signal = tokio::select! {
                        biased;
                        Some(query) = status_rx.recv() => {
                            self.state = ChatAgentState::busy(result_rx, started_at);
                            self.answer_status(query);
                            continue;
                        }
                        result = &mut result_rx => result,
                    };
                    match signal {
                        Ok(BusySignal::Turn(output)) => {
                            self.on_complete(output).await;
                        }
                        Ok(BusySignal::Steered(output)) => {
                            self.on_complete(output).await;
                            // The turn just ended already spent the batch's budget, so this failure
                            // only ends the tick.
                            if let Err(e) = self.refresh_batch().await {
                                tracing::warn!(%self.chat_id, "refresh after steering failed: {e}");
                                self.end_run();
                                continue;
                            }
                            if !self.batch.is_empty() {
                                self.dispatch(&doorbell, StartedBy::Steered).await;
                            }
                        }
                        Ok(BusySignal::WrapUp {
                            summary,
                            context_tokens,
                        }) => {
                            if self.on_wrap_up_done(summary, context_tokens).await.is_err() {
                                self.stand_down();
                            }
                            self.end_run();
                        }
                        Ok(BusySignal::Cancelled) => self.end_run(),
                        Ok(BusySignal::TurnFailed) => {
                            tracing::warn!(%self.chat_id, "Busy task failed");
                            if self.retry.fail_now() {
                                tracing::warn!(
                                    %self.chat_id,
                                    remaining = self.retry.remaining(),
                                    "Re-dispatching after a failed turn"
                                );
                            }
                            self.end_run();
                        }
                        Ok(BusySignal::WrapUpFailed(err)) => {
                            self.engine
                                .runtime
                                .app
                                .event_bus
                                .emit(self.chat_id, AgentEventPayload::WrapUpFailed { error: err });
                            self.conversation.start_new_chapter(None);
                            if self.persist_conversation(&[]).await.is_err() {
                                self.stand_down();
                            }
                            self.end_run();
                        }
                        Err(_) => {
                            tracing::warn!(%self.chat_id, "Busy task panicked");
                            self.end_run();
                        }
                    }
                }
            }
        }
    }

    fn should_wrap_up_by_tokens(&self) -> bool {
        let threshold = self
            .engine
            .runtime
            .app
            .cfg
            .agent
            .context
            .compact_token_threshold;
        threshold > 0 && self.conversation.context_tokens() >= threshold
    }

    async fn start_wrap_up(&mut self, doorbell: &Doorbell) {
        let language = match self.language().await {
            Ok(language) => language,
            Err(e) => {
                tracing::warn!(%self.chat_id, "language lookup failed: {e}");
                self.wrap_up_retry_at = Some(tokio::time::Instant::now() + RETRY_BACKOFF);
                return;
            }
        };
        self.wrap_up_retry_at = None;
        let ctx = self.build_turn_context(WakeEvents::new(vec![]), language);
        let result_rx = self.engine.spawn_wrap_up(
            ctx,
            self.tools.clone(),
            self.conversation.messages_for_wrap_up(),
            doorbell.clone(),
        );
        self.state = ChatAgentState::busy(result_rx, tokio::time::Instant::now());
    }

    async fn idle_tick(
        &mut self,
        doorbell: &Doorbell,
        status_rx: &mut mpsc::UnboundedReceiver<oneshot::Sender<ChatAgentStatus>>,
    ) -> NextStep {
        if let Some(at) = self.retry.owed_at() {
            if self.wait_until(at, status_rx).await {
                return NextStep::Idle;
            }
            self.retry.dispatched();
            return NextStep::Activate(StartedBy::Retry);
        }
        if let Some(at) = self.wrap_up_retry_at
            && self.wait_until(at, status_rx).await
        {
            return NextStep::Idle;
        }
        if self.should_wrap_up_by_tokens() {
            return NextStep::WrapUp;
        }
        if let Some(last_active) = self.conversation.take_restored_last_active()
            && self.conversation.has_unwrapped_content()
        {
            let timeout = jiff::SignedDuration::from_secs(self.idle_timeout().as_secs() as i64);
            if jiff::Timestamp::now().duration_since(last_active) > timeout {
                return NextStep::WrapUp;
            }
        }
        let timeout = self.idle_timeout();
        let deadline = self.schedule.next_deadline(timeout);

        tokio::select! {
            _ = doorbell.notified() => {},
            Some(query) = status_rx.recv() => {
                self.answer_status(query);
                return NextStep::Idle;
            }
            _ = tokio::time::sleep_until(deadline) => {},
        }

        if let Err(e) = self.refresh_batch().await {
            tracing::warn!(%self.chat_id, "refresh_batch failed: {e}");
            self.stand_down();
            return NextStep::Idle;
        }

        let has_pending = !self.batch.is_empty() && self.retry.remaining() > 0;
        match self.schedule.decide(has_pending, timeout) {
            Decision::Ready(started_by) => NextStep::Activate(started_by),
            Decision::Defer => NextStep::Idle,
            Decision::Done => {
                if self.conversation.has_unwrapped_content() {
                    NextStep::WrapUp
                } else {
                    NextStep::Exit
                }
            }
        }
    }

    /// Waits out a retry deadline inside the caller's `select!` branch, so shutdown still cancels it;
    /// a status query arriving during the wait is answered rather than left hanging. `true` means the
    /// tick must end.
    async fn wait_until(
        &mut self,
        at: Instant,
        status_rx: &mut mpsc::UnboundedReceiver<oneshot::Sender<ChatAgentStatus>>,
    ) -> bool {
        tokio::select! {
            biased;
            Some(query) = status_rx.recv() => {
                self.answer_status(query);
                true
            }
            _ = tokio::time::sleep_until(at) => false,
        }
    }

    async fn on_wrap_up_done(&mut self, summary: String, context_tokens: u32) -> Result<()> {
        let wrapped_up_steps = self.conversation.step_count();
        self.engine.runtime.event_bus().emit(
            self.chat_id,
            AgentEventPayload::WrapUpCompleted {
                step_count: wrapped_up_steps as usize,
                context_tokens,
                summary: summary.clone(),
            },
        );
        self.conversation.start_new_chapter(Some(summary));

        self.persist_conversation(&[]).await
    }

    pub fn build_status(&mut self) -> ChatAgentStatus {
        let turn_elapsed = match &self.state {
            ChatAgentState::Busy { started_at, .. } => Some(started_at.elapsed().as_secs_f64()),
            ChatAgentState::Idle => None,
        };
        ChatAgentStatus {
            scheduler: self.schedule.snapshot(),
            step_count: self.conversation.step_count() as usize,
            context_tokens: self.conversation.context_tokens(),
            conversation_msgs: self.conversation.message_count(),
            turn_elapsed_secs: turn_elapsed,
            model: self
                .engine
                .runtime
                .app
                .cfg
                .model_roles
                .default
                .model
                .clone(),
        }
    }

    pub fn answer_status(&mut self, query: oneshot::Sender<ChatAgentStatus>) {
        let _ = query.send(self.build_status());
    }
}
