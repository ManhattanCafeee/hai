use genai::chat::ChatMessage;

use crate::domain::vo::ContextMeta;

#[derive(Clone)]
pub struct ConversationSnapshot {
    /// Accumulated sequence handed to the LLM, injections and tool chains included.
    pub context_messages: Vec<ChatMessage>,
    pub context_meta: ContextMeta,
}
