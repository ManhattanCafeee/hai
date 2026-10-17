use std::collections::HashMap;

use genai::chat::ChatMessage;

use crate::domain::{model::Message, vo::MessageId};

/// What one prompt build produces: the rendered prompt, and this build's contribution to the
/// accumulated context as a single user message.
#[derive(Debug)]
pub struct BuiltContext {
    pub rendered_prompt: String,
    pub messages: Vec<ChatMessage>,
}

/// One turn's context messages: the conversation flow (window) plus the out-of-window reply targets.
#[derive(Debug)]
pub struct ContextMessages {
    /// The conversation flow: this turn's gathered window.
    window: Vec<Message>,
    window_by_id: HashMap<MessageId, usize>,
    /// Reply targets a window message points at but the window does not contain.
    reply: HashMap<MessageId, Message>,
}

/// Where a referenced message came from; this decides the reference rendering strategy.
#[derive(Debug)]
pub enum MessageSource<'a> {
    /// In the window: the message is rendered as its own `<msg>`.
    InWindow(&'a Message),
    /// Out-of-window reply context: the reference is its only presentation.
    Reply(&'a Message),
}

impl ContextMessages {
    pub fn new(window: Vec<Message>, reply: HashMap<MessageId, Message>) -> Self {
        let window_by_id = window.iter().enumerate().map(|(i, m)| (m.id, i)).collect();
        Self {
            window,
            window_by_id,
            reply,
        }
    }

    /// The window messages, each rendered as its own `<msg>`.
    pub fn window(&self) -> &[Message] {
        &self.window
    }

    /// Looks a message up by id, window first, tagged with its source.
    pub fn get(&self, id: MessageId) -> Option<MessageSource<'_>> {
        if let Some(&i) = self.window_by_id.get(&id) {
            return Some(MessageSource::InWindow(&self.window[i]));
        }
        self.reply.get(&id).map(MessageSource::Reply)
    }
}

impl<'a> MessageSource<'a> {
    pub fn message(&self) -> &'a Message {
        match self {
            MessageSource::InWindow(m) | MessageSource::Reply(m) => m,
        }
    }

    pub fn is_in_window(&self) -> bool {
        matches!(self, MessageSource::InWindow(_))
    }
}
