use super::super::types::Messages;
use crate::domain::vo::{ContextMeta, ConversationSnapshot};

/// Conversation state. Everything persistent lives in `ConversationSnapshot` (context
/// messages, the chapter metadata): the execution environment is rebuilt from
/// it on wake, the conversation itself never disappears with the environment. The
/// conversation-level vs chapter-level split is owned by docs/topics/chat-agent.md.
pub(crate) struct Conversation {
    context_messages: Messages,
    chapter: ContextMeta,
    restored_last_active: Option<jiff::Timestamp>,
}

impl Conversation {
    pub fn new() -> Self {
        Self {
            context_messages: Messages::new(Vec::new()),
            chapter: ContextMeta::new(),
            restored_last_active: None,
        }
    }

    pub fn from_snapshot(
        snap: ConversationSnapshot,
        restored_last_active: Option<jiff::Timestamp>,
    ) -> Self {
        Self {
            context_messages: Messages::new(snap.context_messages),
            chapter: snap.context_meta,
            restored_last_active,
        }
    }

    pub fn snapshot(&self) -> ConversationSnapshot {
        ConversationSnapshot {
            context_messages: self.context_messages.to_vec(),
            context_meta: self.chapter,
        }
    }

    pub fn update(&mut self, messages: Messages, step_count: usize, last_prompt_tokens: u32) {
        self.context_messages = messages;
        self.chapter.advance(step_count, last_prompt_tokens);
    }

    pub fn build_full_messages(&self, chunk: Vec<genai::chat::ChatMessage>) -> Messages {
        let mut full = self.context_messages.clone();
        full.extend(chunk);
        full
    }

    /// The chapter's initial state: a chapter with no completed Turn is built whole — backfill
    /// included — while one that has run a Turn continues from the context it holds. A chapter
    /// opened by a wrap-up summary holds that message and is still its first render.
    pub fn is_first_render(&self) -> bool {
        self.turn_count() == 0
    }

    pub fn messages_for_wrap_up(&self) -> Messages {
        self.context_messages.clone()
    }

    pub fn message_count(&self) -> usize {
        self.context_messages.len()
    }

    pub fn context_tokens(&self) -> u32 {
        self.chapter.tokens
    }

    pub fn step_count(&self) -> u64 {
        self.chapter.step_count
    }

    pub fn turn_count(&self) -> u64 {
        self.chapter.turn_count
    }

    pub fn has_unwrapped_content(&self) -> bool {
        self.chapter.is_non_empty()
    }

    pub fn take_restored_last_active(&mut self) -> Option<jiff::Timestamp> {
        self.restored_last_active.take()
    }

    pub fn start_new_chapter(&mut self, summary: Option<String>) {
        self.context_messages = match summary {
            Some(s) => Messages::new(vec![genai::chat::ChatMessage::user(s)]),
            None => Messages::new(Vec::new()),
        };
        self.chapter = ContextMeta::new();
    }
}
