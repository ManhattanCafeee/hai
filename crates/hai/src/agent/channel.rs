//! One channel as the agent sees it: `Channel` is the outbound side (send, files, typing, and what
//! that channel declares), `ChannelAccess` the inbound one (who may enter, and how host copy is
//! delivered). The platform layer implements both — an agent module holds a `Channel` and never a
//! platform type. Acting on a chat goes through `ChannelHandle`, which binds the chat the agent
//! belongs to.

use std::{fmt::Debug, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::{
    agent::web::WebUrl,
    domain::vo::{AccountId, ChannelKey, ChatId, MessageId, TopicId},
    error::Result,
};

/// The channel's own identity, as the agent context renders it.
#[derive(Debug, Clone)]
pub struct ChannelProfile {
    pub account_id: AccountId,
    pub username: String,
    pub name: String,
}

/// What one message points at inside the chat it goes to.
#[derive(Debug)]
pub struct ReplyTo {
    pub topic_id: Option<TopicId>,
    pub reference: Option<MessageId>,
    /// A part of that message to point at, copied verbatim from it.
    pub quote: Option<String>,
}

/// The bytes of one outbound message's media, or where the platform should fetch them.
#[derive(Debug, Clone)]
pub enum OutboundSource {
    Bytes(Vec<u8>),
    Url(WebUrl),
}

#[derive(Debug, Clone)]
pub enum OutboundPayload {
    Text {
        content: String,
    },
    Photo {
        source: OutboundSource,
        content: Option<String>,
    },
    Voice {
        bytes: Vec<u8>,
        content: Option<String>,
        /// What the speech model was asked for; kept on the attachment row.
        prompt: String,
    },
    Document {
        source: OutboundSource,
        filename: String,
        content: Option<String>,
    },
}

/// One outgoing message with its chat resolved: what the platform layer is handed to send.
#[derive(Debug)]
pub struct OutboundMessage {
    pub chat_id: ChatId,
    pub reply: ReplyTo,
    pub payload: OutboundPayload,
}

/// One chat on the channel the agent runs on. The chat is bound here, so every verb that acts on it
/// already knows which one, and no caller can aim a send or an indicator at another chat.
#[derive(Debug, Clone)]
pub struct ChannelHandle {
    channel: Arc<dyn Channel>,
    chat_id: ChatId,
}

impl ChannelHandle {
    pub fn new(channel: Arc<dyn Channel>, chat_id: ChatId) -> Self {
        Self { channel, chat_id }
    }

    /// The chat this handle acts on.
    pub fn chat_id(&self) -> ChatId {
        self.chat_id
    }

    pub fn key(&self) -> &ChannelKey {
        self.channel.key()
    }

    pub fn profile(&self) -> ChannelProfile {
        self.channel.profile()
    }

    pub fn capabilities(&self) -> ChannelCapabilities {
        self.channel.capabilities()
    }

    /// Sends something the agent produced into this handle's chat.
    pub async fn send(&self, reply: ReplyTo, payload: OutboundPayload) -> Result<()> {
        self.channel
            .send(OutboundMessage {
                chat_id: self.chat_id,
                reply,
                payload,
            })
            .await
    }

    /// Starts the platform's typing indicator for this handle's chat and hands back the guard
    /// holding it alive; the platform answers `None` when it shows no indicator.
    #[must_use = "dropping the guard ends the indicator it started"]
    pub async fn start_typing(&self) -> Option<Box<dyn TypingGuard>> {
        self.channel.start_typing(self.chat_id).await
    }

    /// Downloads a platform file's bytes, for attachment analysis.
    pub async fn download_file(&self, file_id: &str) -> Result<Vec<u8>> {
        self.channel.download_file(file_id).await
    }

    /// A platform file's URL, for multimodal analysis.
    pub async fn file_url(&self, file_id: &str) -> Result<String> {
        self.channel.file_url(file_id).await
    }
}

/// One way a message can go out; a platform serves a subset of these.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum OutboundKind {
    Text,
    Voice,
    File,
}

/// One argument a form takes.
#[derive(Debug, Clone, Copy)]
pub struct OutboundField {
    pub name: &'static str,
    pub description: &'static str,
}

/// What a form needs this build to have configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildCapability {
    None,
    Speech,
}

/// One way a platform puts something in a chat: the kind it sends, the arguments that form takes,
/// and what the build must have for the form to exist.
#[derive(Debug, Clone, Copy)]
pub struct OutboundForm {
    pub kind: OutboundKind,
    pub fields: &'static [OutboundField],
    pub requires: BuildCapability,
}

/// What one platform can put in a chat: the forms it serves and the reply arguments it accepts; the
/// send tool's surface is pruned from it, so a platform is never asked for what it cannot do.
#[derive(Debug, Clone, Copy)]
pub struct ChannelCapabilities {
    pub forms: &'static [OutboundForm],
    /// A message can point at another one.
    pub reference: bool,
    /// A message can quote a part of the one it points at.
    pub quote: bool,
    /// The largest single upload this platform takes; a bigger local file is refused before it is read.
    pub max_media_bytes: u64,
}

/// A live "typing" indicator: the value's lifetime *is* the indicator's — dropping it ends what the
/// platform started, and how the platform keeps it alive between those two points is that platform's
/// own business.
pub trait TypingGuard: Send + 'static {}

/// What the agent needs from the channel it runs on: its identity, what it offers, and the few
/// things it can do. A platform implements the adapter (`platform::adapter::ChannelAdapter`); the
/// platform layer answers this trait around it, so the agent never reaches a vendor API or the
/// database. One value is one configured channel, shared by every chat on it: the agent acts on a
/// chat through `ChannelHandle`, and a verb that takes no chat acts on the channel itself.
#[async_trait::async_trait]
pub trait Channel: Debug + Send + Sync + 'static {
    /// The key this channel instance is stored under, spelled once where it is built.
    fn key(&self) -> &ChannelKey;
    fn profile(&self) -> ChannelProfile;
    fn capabilities(&self) -> ChannelCapabilities;
    /// Sends something the agent produced and records what left.
    async fn send(&self, msg: OutboundMessage) -> Result<()>;
    /// Downloads a platform file's bytes, for attachment analysis.
    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>>;
    /// A platform file's URL, for multimodal analysis.
    async fn file_url(&self, file_id: &str) -> Result<String>;
    /// Starts the platform's typing indicator for this chat and hands back the guard holding it
    /// alive. `None` when the platform shows no such indicator, or when it could not be started —
    /// the indicator is decoration, so a failure is logged and never reaches the caller.
    #[must_use = "dropping the guard ends the indicator it started"]
    async fn start_typing(&self, chat_id: ChatId) -> Option<Box<dyn TypingGuard>>;
}

/// What an inbound path needs from the channel's platform: whether a chat may talk here, and how to
/// deliver copy the host composed. The inbound tail is the only reader, so this is exactly the
/// adapter's two inbound methods and nothing else.
#[async_trait::async_trait]
pub trait ChannelAccess: Debug + Send + Sync + 'static {
    /// Whether this chat may use the channel; the allowlist spelling is the platform's own.
    fn admits(&self, chat_external_id: &str) -> bool;
    /// Delivers host copy (a greeting, an unauthorized notice); never stored.
    async fn notice(&self, chat_external_id: &str, text: &str) -> Result<()>;
}
