use std::collections::HashSet;

use tokio::time::Instant;

use super::{
    super::{
        event::{WakeEvent, WakeEvents, WakeReason},
        turn::TurnContext,
    },
    ChatAgent, ChatAgentState,
};
use crate::{
    agent::{
        context,
        runtime::{event::Doorbell, types::TurnOutput},
    },
    agentcore::Language,
    domain::{
        model::Message,
        vo::{AgentEventPayload, BatchComposition, MessageId, StartedBy, TurnNumber},
    },
    error::Result,
};

impl ChatAgent {
    /// Starts a turn over the batch as it stands: the batch is the input, so a failure before the
    /// turn starts leaves it untouched for the next attempt.
    pub async fn dispatch(&mut self, doorbell: &Doorbell, started_by: StartedBy) {
        // The batch this turn reads accounts for everything that had arrived by now; anything set
        // after this line belongs to the next turn.
        doorbell.take_signal();

        let language = match self.language().await {
            Ok(language) => language,
            Err(e) => {
                tracing::warn!(%self.chat_id, "language lookup failed: {e}");
                return self.stand_down();
            }
        };
        let ctx = self.build_turn_context(self.batch_reasons(), language);

        let messages = match self.gather_batch().await {
            Ok(messages) => messages,
            Err(e) => {
                tracing::warn!(%self.chat_id, "gather_batch failed: {e}");
                return self.stand_down();
            }
        };

        let gathered = messages.len();
        let is_first = self.conversation.is_first_render();
        let new_message_ids: HashSet<MessageId> = self
            .batch
            .iter()
            .filter_map(|entry| entry.message_id())
            .collect();
        let built = match context::build_prompt(&ctx, &messages, is_first, &new_message_ids).await {
            Ok(built) => built,
            Err(e) => {
                tracing::warn!(%self.chat_id, "build_prompt failed: {e}");
                return self.stand_down();
            }
        };

        let messages = self.conversation.build_full_messages(built.messages);
        let turn_number = TurnNumber::from((self.conversation.turn_count() + 1) as usize);

        let reason = ctx
            .events
            .strongest()
            .expect("dispatch runs only for a batch the loop woke on");

        let composition = BatchComposition::of(&self.batch);

        self.engine.runtime.event_bus().emit(
            self.chat_id,
            AgentEventPayload::TurnStarted {
                turn: turn_number,
                reason,
                msg_count: gathered,
                full_prompt: built.rendered_prompt.clone(),
                started_by: Some(started_by),
                composition: Some(composition),
            },
        );

        let result_rx = self.engine.spawn_turn(
            ctx,
            self.tools.clone(),
            messages,
            doorbell.clone(),
            turn_number,
        );

        self.state = ChatAgentState::busy(result_rx, Instant::now());
    }

    /// Commits the turn: the changes it consumed leave the accumulation in the same step as the
    /// snapshot, so a turn that never reaches here leaves both untouched.
    pub async fn on_complete(&mut self, output: TurnOutput) {
        if output.has_spoken {
            self.schedule.on_spoke();
        }

        let last_prompt_tokens = output.steps.last().map(|t| t.prompt_tokens).unwrap_or(0);
        self.conversation
            .update(output.messages, output.steps.len(), last_prompt_tokens);

        let consumed: Vec<uuid::Uuid> = self.batch.iter().map(|entry| entry.id).collect();
        if self.persist_conversation(&consumed).await.is_ok() {
            self.batch.clear();
        }

        self.retry.reopen();
        self.end_run();
    }

    fn batch_reasons(&self) -> WakeEvents {
        WakeEvents::new(
            self.batch
                .iter()
                .map(|entry| WakeEvent::new(WakeReason::of_entry(entry, self.chat_type)))
                .collect(),
        )
    }

    pub fn build_turn_context(&self, events: WakeEvents, language: Language) -> TurnContext {
        TurnContext {
            tool: self.tool.clone(),
            app: self.engine.runtime.app.clone(),
            chat_type: self.chat_type,
            language,
            events,
        }
    }

    /// The language the chat reads its persona and replies in; a failure is the caller's to handle.
    pub(crate) async fn language(&self) -> Result<Language> {
        self.engine
            .runtime
            .app
            .settings
            .language(self.chat_id)
            .await
    }

    pub async fn gather_batch(&self) -> Result<Vec<Message>> {
        let services = self.engine.runtime.services();

        let mut ids: Vec<MessageId> = self
            .batch
            .iter()
            .filter_map(|entry| entry.message_id())
            .collect();
        ids.sort_unstable();
        ids.dedup();

        let mut messages = services.message.get_messages_by_ids(&ids).await?;

        if self.conversation.is_first_render() {
            let cap = self.engine.runtime.cfg().agent.context.context_seed_cap;
            messages.extend(services.message.recent_asc(self.chat_id, cap).await?);
            messages.sort_by_key(|message| message.id);
            messages.dedup_by_key(|message| message.id);
        }

        Ok(messages)
    }
}
