use std::collections::HashMap;

use crate::{
    domain::{
        model::{Attachment, Message, MessageRole, NewAttachment, NewMessage},
        repo::Repos,
        vo::{
            AccountId, AttachmentId, ChatId, LlmMessageMeta, MessageId, MessageMeta,
            PlatformMessageMeta, TopicId,
        },
    },
    error::Result,
};

/// One message the agent sends. Its platform extras travel with it so the row records what actually
/// left — an outgoing reply that pointed at an excerpt included.
pub struct NewAgentMessage {
    pub chat_id: ChatId,
    pub account_id: Option<AccountId>,
    pub text: String,
    pub attachments: Vec<NewAttachment>,
    pub model: String,
    pub platform_meta: Option<PlatformMessageMeta>,
    pub reply_to_id: Option<MessageId>,
    pub external_id: Option<String>,
    pub sent_at: Option<jiff::Timestamp>,
    pub topic_id: Option<TopicId>,
}

pub(crate) struct UpsertMessageParams {
    pub chat_id: ChatId,
    pub external_id: Option<String>,
    pub role: MessageRole,
    pub text: String,
    pub attachments: Vec<NewAttachment>,
    pub account_id: Option<AccountId>,
    pub reply_to_id: Option<MessageId>,
    pub meta: serde_json::Value,
    pub sent_at: Option<jiff::Timestamp>,
    pub topic_id: Option<TopicId>,
}

#[derive(Debug)]
pub struct MessageService {
    repos: Repos,
}

impl MessageService {
    pub fn new(repos: Repos) -> Self {
        Self { repos }
    }

    async fn upsert_by_external_id(&self, params: UpsertMessageParams) -> Result<Message> {
        if let Some(ext_id) = &params.external_id
            && let Some(existing) = self
                .repos
                .message
                .find_by_chat_external_id(params.chat_id, ext_id)
                .await?
        {
            return self
                .repos
                .message
                .refresh(
                    &existing,
                    params.text,
                    params.meta,
                    existing.edited_at,
                    &params.attachments,
                )
                .await;
        }
        self.repos
            .message
            .create(NewMessage {
                chat_id: params.chat_id,
                account_id: params.account_id,
                role: params.role,
                text: params.text,
                attachments: params.attachments,
                topic_id: params.topic_id,
                reply_to_id: params.reply_to_id,
                external_id: params.external_id,
                meta: params.meta,
                sent_at: params.sent_at,
                edited_at: None,
            })
            .await
    }

    pub async fn save_agent_message(&self, msg: NewAgentMessage) -> Result<Message> {
        let meta = MessageMeta {
            platform: msg.platform_meta,
            llm: Some(LlmMessageMeta {
                model: msg.model,
                reasoning: None,
            }),
        };
        self.upsert_by_external_id(UpsertMessageParams {
            chat_id: msg.chat_id,
            external_id: msg.external_id,
            role: MessageRole::Assistant,
            text: msg.text,
            attachments: msg.attachments,
            account_id: msg.account_id,
            reply_to_id: msg.reply_to_id,
            meta: serde_json::to_value(&meta)?,
            sent_at: msg.sent_at,
            topic_id: msg.topic_id,
        })
        .await
    }

    /// Everything a quote may be copied from: the message's own text plus each attachment's caption,
    /// because that is what the context renders as the message's content.
    pub async fn quoted_content(&self, message: &Message) -> Result<String> {
        let attachments = self.repos.attachment.by_message_ids(&[message.id]).await?;
        Ok(message.content(&attachments))
    }

    pub async fn get_message_by_id(&self, id: MessageId) -> Result<Option<Message>> {
        self.repos.message.by_id(id).await
    }

    pub async fn get_messages_by_ids(&self, ids: &[MessageId]) -> Result<Vec<Message>> {
        self.repos.message.by_ids(ids).await
    }

    pub async fn recent_asc(&self, chat_id: ChatId, limit: i64) -> Result<Vec<Message>> {
        self.repos.message.recent_asc(chat_id, limit).await
    }

    pub async fn has_newer_from_user(&self, chat_id: ChatId, id: MessageId) -> Result<bool> {
        self.repos.message.has_newer_from_user(chat_id, id).await
    }

    pub async fn update_meta(&self, id: MessageId, meta: Option<serde_json::Value>) -> Result<()> {
        self.repos.message.update_meta(id, meta).await
    }

    pub async fn attachments_by_messages(
        &self,
        ids: &[MessageId],
    ) -> Result<HashMap<MessageId, Vec<Attachment>>> {
        let rows = self.repos.attachment.by_message_ids(ids).await?;
        let mut grouped: HashMap<MessageId, Vec<Attachment>> = HashMap::new();
        for row in rows {
            grouped.entry(row.message_id).or_default().push(row);
        }
        Ok(grouped)
    }

    pub async fn attachment_by_id(&self, id: AttachmentId) -> Result<Option<Attachment>> {
        self.repos.attachment.by_id(id).await
    }
}
