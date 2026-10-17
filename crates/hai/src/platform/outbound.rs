//! What the platform layer does around one send: turn the agent's internal target into the platform's
//! own address, hold a reference to the same rules the send tool advertises, and record the row that
//! says the message left. A platform's adapter does none of this.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    agent::channel::{
        Channel, ChannelCapabilities, ChannelProfile, OutboundMessage, OutboundPayload, ReplyTo,
        TypingGuard,
    },
    domain::{
        model::{Chat, ChatType, NewAttachment, Platform},
        service::{NewAgentMessage, Services},
        vo::{AccountId, ChannelKey, ChatId, MessageId, attachment_id},
    },
    error::{AppError, Result},
    platform::adapter::{ChannelAdapter, SentAttachment, SentMessage, WireMessage, WireReference},
};

/// The channel as the agent sees it: the adapter underneath, plus the store keeping.
pub(crate) struct OutboundRecorder {
    adapter: Arc<dyn ChannelAdapter>,
    services: Services,
    bot_account_id: AccountId,
    model: String,
}

impl OutboundRecorder {
    pub(crate) fn new(adapter: Arc<dyn ChannelAdapter>, services: Services, model: String) -> Self {
        // The adapter already answers this; asking it once here keeps a second copy out of the builder.
        let bot_account_id = adapter.profile().account_id;
        Self {
            adapter,
            services,
            bot_account_id,
            model,
        }
    }

    /// The chat row behind one internal id; an id without a row is a bug the caller must see.
    async fn resolve_chat(&self, chat_id: ChatId) -> Result<Chat> {
        self.services
            .platform
            .get_chat_by_id(chat_id)
            .await?
            .ok_or_else(|| AppError::message(format!("Chat not found for internal id: {chat_id}")))
    }

    /// The reference as the platform needs it: the target message's own id and, when the sender
    /// quoted the message, the verbatim excerpt.
    async fn resolve_reference(
        &self,
        chat: &Chat,
        reference: Option<MessageId>,
        quote: Option<&str>,
    ) -> Result<Option<WireReference>> {
        let Some(msg_id) = reference else {
            return Ok(None);
        };
        let Some(msg) = self.services.message.get_message_by_id(msg_id).await? else {
            return Err(AppError::message(format!(
                "no message {msg_id} to point at"
            )));
        };
        if msg.chat_id != chat.id {
            return Err(AppError::message(format!(
                "message {msg_id} belongs to another chat"
            )));
        }

        if let Some(quote) = quote {
            let content = self.services.message.quoted_content(&msg).await?;
            if !excerpt_is_verbatim(&content, quote) {
                return Err(AppError::message(format!(
                    "quote is not a verbatim part of message {msg_id}"
                )));
            }
        } else if chat.chat_type == ChatType::Private
            && !self
                .services
                .message
                .has_newer_from_user(chat.id, msg_id)
                .await?
        {
            // In a one-to-one chat a reference to the newest thing the person said is the whole
            // conversation already; pointing anywhere else, or quoting a part, still says something.
            return Ok(None);
        }

        let external_id = msg.external_id.ok_or_else(|| {
            AppError::message(format!("message {msg_id} has no platform id to point at"))
        })?;
        Ok(Some(WireReference {
            external_id,
            excerpt: quote.map(str::to_owned),
        }))
    }

    /// The row that records what left: one message, and one attachment row per file it carried.
    async fn record(
        &self,
        chat_id: ChatId,
        reply: &ReplyTo,
        text: String,
        sent: SentMessage,
    ) -> Result<()> {
        let row = sent_row(
            self.adapter.platform(),
            self.bot_account_id,
            &self.model,
            chat_id,
            reply,
            text,
            sent,
        );
        self.services.message.save_agent_message(row).await?;
        Ok(())
    }
}

/// The message row one accepted send becomes.
fn sent_row(
    platform: Platform,
    account_id: AccountId,
    model: &str,
    chat_id: ChatId,
    reply: &ReplyTo,
    text: String,
    sent: SentMessage,
) -> NewAgentMessage {
    let SentMessage {
        external_id,
        sent_at,
        reference_sent,
        platform_meta,
        attachments,
    } = sent;
    let attachments = attachments
        .into_iter()
        .enumerate()
        .map(|(ordinal, attachment)| {
            let ordinal = i32::try_from(ordinal).expect("a message carries fewer than 2^31 files");
            attachment_row(platform, chat_id, &external_id, ordinal, attachment)
        })
        .collect();

    NewAgentMessage {
        chat_id,
        account_id: Some(account_id),
        text,
        attachments,
        model: model.to_owned(),
        platform_meta,
        topic_id: reply.topic_id,
        reply_to_id: reference_sent.then_some(reply.reference).flatten(),
        external_id: Some(external_id),
        sent_at: Some(sent_at),
    }
}

/// One attachment row for what the platform reported; a send that returned no download handle keeps
/// the row's own occurrence id, which no platform handle can collide with.
fn attachment_row(
    platform: Platform,
    chat_id: ChatId,
    external_id: &str,
    ordinal: i32,
    attachment: SentAttachment,
) -> NewAttachment {
    let file_id = attachment.file_id.unwrap_or_else(|| {
        attachment_id(platform.as_str(), chat_id, external_id, ordinal).to_string()
    });
    NewAttachment::new(
        platform,
        ordinal,
        chat_id,
        external_id,
        attachment.kind,
        attachment.file_unique_id,
        file_id,
        attachment.caption,
        attachment.meta,
    )
}

/// Whether an excerpt may be sent: Telegram refuses a quote the message does not contain, so a quote
/// that cannot go out is an error rather than a silently dropped field.
fn excerpt_is_verbatim(content: &str, quote: &str) -> bool {
    !quote.is_empty() && content.contains(quote)
}

/// The message's own words: its text, or the line that went with its media.
fn text_of(payload: &OutboundPayload) -> String {
    match payload {
        OutboundPayload::Text { content } => content.clone(),
        OutboundPayload::Photo { content, .. }
        | OutboundPayload::Voice { content, .. }
        | OutboundPayload::Document { content, .. } => content.clone().unwrap_or_default(),
    }
}

impl std::fmt::Debug for OutboundRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutboundRecorder").finish_non_exhaustive()
    }
}

#[async_trait]
impl Channel for OutboundRecorder {
    fn key(&self) -> &ChannelKey {
        self.adapter.key()
    }

    fn profile(&self) -> ChannelProfile {
        self.adapter.profile()
    }

    fn capabilities(&self) -> ChannelCapabilities {
        self.adapter.capabilities()
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let OutboundMessage {
            chat_id,
            reply,
            payload,
        } = msg;
        let chat = self.resolve_chat(chat_id).await?;
        let reference = self
            .resolve_reference(&chat, reply.reference, reply.quote.as_deref())
            .await?;
        if let Some(topic_id) = reply.topic_id {
            // A topic belongs to one chat; linking another chat's topic would file this message
            // under a record it is not part of.
            self.services.topic.get(chat_id, topic_id).await?;
        }
        let text = text_of(&payload);
        let sent = self
            .adapter
            .send(WireMessage {
                chat_external_id: chat.external_id,
                reference,
                payload,
            })
            .await?;
        self.record(chat_id, &reply, text, sent).await
    }

    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>> {
        self.adapter.download_file(file_id).await
    }

    async fn file_url(&self, file_id: &str) -> Result<String> {
        self.adapter.file_url(file_id).await
    }

    async fn start_typing(&self, chat_id: ChatId) -> Option<Box<dyn TypingGuard>> {
        match self.resolve_chat(chat_id).await {
            Ok(chat) => self.adapter.start_typing(&chat.external_id),
            Err(e) => {
                tracing::warn!(%chat_id, error = %e, "typing indicator skipped");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::{AttachmentKind, AttachmentMeta, TopicId};

    fn reply(topic_id: Option<TopicId>, reference: Option<MessageId>) -> ReplyTo {
        ReplyTo {
            topic_id,
            reference,
            quote: None,
        }
    }

    fn sent(reference_sent: bool, attachments: Vec<SentAttachment>) -> SentMessage {
        SentMessage {
            external_id: "77".to_owned(),
            sent_at: jiff::Timestamp::from_second(1_700_000_000).expect("a valid second"),
            reference_sent,
            platform_meta: None,
            attachments,
        }
    }

    /// A reference the platform dropped is not recorded as one.
    #[test]
    fn a_dropped_reference_is_not_recorded() {
        let reply_to = reply(None, Some(MessageId(3)));
        let row = sent_row(
            Platform::Telegram,
            AccountId(1),
            "a-model",
            ChatId(9),
            &reply_to,
            String::new(),
            sent(false, Vec::new()),
        );
        assert_eq!(row.reply_to_id, None);
    }

    /// A send the platform answered without a file handle keeps the row's own occurrence id, which
    /// no platform handle can collide with.
    #[test]
    fn an_attachment_without_a_handle_falls_back_to_its_occurrence_id() {
        let reply_to = reply(None, None);
        let row = sent_row(
            Platform::Telegram,
            AccountId(1),
            "a-model",
            ChatId(9),
            &reply_to,
            String::new(),
            sent(
                false,
                vec![SentAttachment {
                    kind: AttachmentKind::Voice,
                    file_unique_id: None,
                    file_id: None,
                    caption: Some("the line".to_owned()),
                    meta: AttachmentMeta::default(),
                }],
            ),
        );

        assert_eq!(row.attachments.len(), 1);
        let attachment = &row.attachments[0];
        assert_eq!(attachment.kind, AttachmentKind::Voice);
        assert_eq!(attachment.ordinal, 0);
        assert_eq!(attachment.platform, Platform::Telegram);
        assert_eq!(attachment.caption.as_deref(), Some("the line"));
        assert_eq!(
            attachment.file_id,
            attachment_id(Platform::Telegram.as_str(), ChatId(9), "77", 0).to_string()
        );
        assert_eq!(
            attachment.id,
            attachment_id(Platform::Telegram.as_str(), ChatId(9), "77", 0)
        );
    }

    /// A handle the platform did return is what the row keeps.
    #[test]
    fn an_attachment_with_a_handle_keeps_it() {
        let row = sent_row(
            Platform::Telegram,
            AccountId(1),
            "a-model",
            ChatId(9),
            &reply(None, None),
            String::new(),
            sent(
                false,
                vec![SentAttachment {
                    kind: AttachmentKind::Document,
                    file_unique_id: Some("uniq".to_owned()),
                    file_id: Some("handle".to_owned()),
                    caption: None,
                    meta: AttachmentMeta::default(),
                }],
            ),
        );

        assert_eq!(row.attachments[0].file_id, "handle");
        assert_eq!(row.attachments[0].file_unique_id.as_deref(), Some("uniq"));
    }

    #[test]
    fn a_quote_must_be_a_verbatim_part_of_its_message() {
        let content = "the whole long original";
        assert!(excerpt_is_verbatim(content, "whole long"));
        assert!(excerpt_is_verbatim(content, content));

        for bad in ["", "not in there", "whole  long"] {
            assert!(!excerpt_is_verbatim(content, bad), "{bad:?} must not pass");
        }
    }
}
