//! The platform layer's own contract: one adapter per platform. Only this layer names it — the agent
//! speaks `agent::channel::{Channel, ChannelAccess}` and never sees a vendor type.

use std::fmt::Debug;

use async_trait::async_trait;

use crate::{
    agent::channel::{
        ChannelAccess, ChannelCapabilities, ChannelProfile, OutboundPayload, TypingGuard,
    },
    domain::{
        model::Platform,
        vo::{AttachmentKind, AttachmentMeta, ChannelKey, PlatformMessageMeta},
    },
    error::Result,
};

/// Reads updates into arrivals and sends what the agent asks for. It answers the inbound contract
/// the agent's tail reads (`ChannelAccess`), so whatever the adapter hands the tail provably
/// satisfies it. Everything shared — admission, ingest, recording a sent message, resolving a
/// reference — lives beside it in this layer, never inside it.
#[async_trait]
pub(crate) trait ChannelAdapter: ChannelAccess + Debug + Send + Sync + 'static {
    fn platform(&self) -> Platform;
    /// The key this channel instance is stored under, spelled once where it is built.
    fn key(&self) -> &ChannelKey;
    fn profile(&self) -> ChannelProfile;
    fn capabilities(&self) -> ChannelCapabilities;
    /// Sends one message the agent produced; the receipt is what the store records.
    async fn send(&self, msg: WireMessage) -> Result<SentMessage>;
    /// Starts this platform's typing indicator for a chat and hands back the guard that keeps it
    /// alive; `None` when the platform shows none or the platform's own id could not be read.
    #[must_use = "dropping the guard ends the indicator it started"]
    fn start_typing(&self, chat_external_id: &str) -> Option<Box<dyn TypingGuard>>;
    /// Downloads a platform file's bytes, for attachment analysis.
    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>>;
    /// A platform file's URL, for multimodal analysis.
    async fn file_url(&self, file_id: &str) -> Result<String>;
}

/// What a platform reports after accepting a message: what the store needs to record that it left.
#[derive(Debug)]
pub(crate) struct SentMessage {
    pub external_id: String,
    pub sent_at: jiff::Timestamp,
    /// The reference actually went out; a platform may drop one for its own reasons.
    pub reference_sent: bool,
    /// The platform extras of the sent message, in the shape an incoming one carries.
    pub platform_meta: Option<PlatformMessageMeta>,
    pub attachments: Vec<SentAttachment>,
}

#[derive(Debug)]
pub(crate) struct SentAttachment {
    pub kind: AttachmentKind,
    pub file_unique_id: Option<String>,
    /// The download handle; a platform that returned none leaves it `None`, and the row keeps its
    /// own derived occurrence id instead.
    pub file_id: Option<String>,
    pub caption: Option<String>,
    pub meta: AttachmentMeta,
}

/// One message to send, already resolved: the chat's own id, the reference resolved to the
/// platform's id, and the payload the agent built.
#[derive(Debug)]
pub(crate) struct WireMessage {
    pub chat_external_id: String,
    pub reference: Option<WireReference>,
    pub payload: OutboundPayload,
}

#[derive(Debug)]
pub(crate) struct WireReference {
    pub external_id: String,
    pub excerpt: Option<String>,
}
