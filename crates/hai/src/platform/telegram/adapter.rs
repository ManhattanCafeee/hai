use std::fmt;

use teloxide::{
    Bot,
    payloads::{SendDocumentSetters, SendPhotoSetters, SendVoiceSetters},
    prelude::Requester,
    types::{ChatId as TgChatId, InputFile, ReplyParameters},
};

use super::{
    config::TelegramConfig,
    forms::{MAX_UPLOAD_BYTES, TELEGRAM_FORMS},
    receipt::{SentMedia, receipt, sent_attachment},
    reply,
    service::{self, TelegramFiles, TelegramService, telegram_err},
    typing::TelegramTyping,
};
use crate::{
    agent::channel::{
        ChannelAccess, ChannelCapabilities, ChannelProfile, OutboundPayload, OutboundSource,
        TypingGuard,
    },
    domain::{
        model::Platform,
        vo::{AccountId, ChannelKey},
    },
    error::{AppError, Result},
    platform::adapter::{ChannelAdapter, SentMessage, WireMessage},
};

/// One media request: the caption when there is one, the reply parameters when there are any, then
/// the send. The three teloxide setters share no trait, so the shared part is a macro.
macro_rules! media_request {
    ($request:expr, $content:expr, $reply_params:expr) => {{
        let mut request = $request;
        if let Some(content) = $content {
            request = request.caption(TelegramAdapter::truncate_caption(content));
        }
        if let Some(params) = $reply_params {
            request = request.reply_parameters(params);
        }
        request.await.map_err(telegram_err)?
    }};
}

/// The Telegram adapter: one channel's own settings, its client, and the send helpers that speak the
/// Bot API. Nothing here reads the store or the process container.
pub(super) struct TelegramAdapter {
    bot: Bot,
    srv: TelegramService,
    files: TelegramFiles,
    key: ChannelKey,
    account_id: AccountId,
    name: String,
    username: String,
    allowed_chat_ids: Vec<i64>,
    rich_message: bool,
}

impl TelegramAdapter {
    pub(super) async fn new(
        bot: Bot,
        key: ChannelKey,
        cfg: TelegramConfig,
        account_id: AccountId,
    ) -> Result<Self> {
        let me = bot.get_me().await.map_err(telegram_err)?;
        let name = bot.get_my_name().await.map_err(telegram_err)?.name;
        let srv = TelegramService::new(bot.clone());
        Ok(Self {
            files: TelegramFiles::new(srv.clone()),
            srv,
            bot,
            key,
            account_id,
            name,
            username: me.username.clone().unwrap_or_default(),
            allowed_chat_ids: cfg.allowed_chat_ids,
            rich_message: cfg.rich_message,
        })
    }

    /// The Telegram chat id behind one of the platform's own chat ids.
    fn chat_id(chat_external_id: &str) -> Result<TgChatId> {
        chat_external_id.parse::<i64>().map(TgChatId).map_err(|e| {
            AppError::message(format!("invalid Telegram chat id {chat_external_id}: {e}"))
        })
    }

    /// Telegram rejects a photo caption longer than 1024 characters, so it is
    /// truncated before sending.
    fn truncate_caption(caption: &str) -> String {
        const CAPTION_MAX: usize = 1024;
        caption.chars().take(CAPTION_MAX).collect()
    }

    async fn send_with_rich_fallback(
        &self,
        content: &str,
        cid: TgChatId,
        reply_params: Option<&ReplyParameters>,
    ) -> Result<teloxide::types::Message> {
        match self.srv.send_rich_message(cid, content, reply_params).await {
            Ok(msg) => Ok(msg),
            Err(e) => {
                tracing::warn!(chat_id = cid.0, error = %e, "rich message send failed; falling back to text");
                service::send_markdown_with_fallback(&self.bot, cid, content, reply_params).await
            }
        }
    }

    async fn send_text(
        &self,
        cid: TgChatId,
        content: &str,
        reply_params: Option<ReplyParameters>,
    ) -> Result<teloxide::types::Message> {
        if self.rich_message {
            self.send_with_rich_fallback(content, cid, reply_params.as_ref())
                .await
        } else {
            service::send_markdown_with_fallback(&self.bot, cid, content, reply_params.as_ref())
                .await
        }
    }

    async fn send_voice(
        &self,
        cid: TgChatId,
        bytes: Vec<u8>,
        content: Option<&str>,
        reply_params: Option<ReplyParameters>,
    ) -> Result<teloxide::types::Message> {
        let request = self.bot.send_voice(cid, InputFile::memory(bytes));
        Ok(media_request!(request, content, reply_params))
    }

    async fn send_photo(
        &self,
        cid: TgChatId,
        source: OutboundSource,
        content: Option<&str>,
        reply_params: Option<ReplyParameters>,
    ) -> Result<teloxide::types::Message> {
        let request = self.bot.send_photo(cid, input_file(source));
        Ok(media_request!(request, content, reply_params))
    }

    async fn send_document(
        &self,
        cid: TgChatId,
        source: OutboundSource,
        filename: String,
        content: Option<&str>,
        reply_params: Option<ReplyParameters>,
    ) -> Result<teloxide::types::Message> {
        let request = self
            .bot
            .send_document(cid, input_file(source).file_name(filename));
        Ok(media_request!(request, content, reply_params))
    }
}

impl fmt::Debug for TelegramAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelegramAdapter").finish()
    }
}

#[async_trait::async_trait]
impl ChannelAccess for TelegramAdapter {
    fn admits(&self, chat_external_id: &str) -> bool {
        chat_external_id
            .parse::<i64>()
            .is_ok_and(|id| self.allowed_chat_ids.contains(&id))
    }

    /// One message the host composes itself: delivered, never stored, and never pointing at anything.
    async fn notice(&self, chat_external_id: &str, text: &str) -> Result<()> {
        let cid = Self::chat_id(chat_external_id)?;
        service::send_markdown_with_fallback(&self.bot, cid, text, None).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn platform(&self) -> Platform {
        Platform::Telegram
    }

    fn key(&self) -> &ChannelKey {
        &self.key
    }

    fn profile(&self) -> ChannelProfile {
        ChannelProfile {
            account_id: self.account_id,
            name: self.name.clone(),
            username: self.username.clone(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            forms: &TELEGRAM_FORMS,
            reference: true,
            quote: true,
            max_media_bytes: MAX_UPLOAD_BYTES,
        }
    }

    async fn send(&self, msg: WireMessage) -> Result<SentMessage> {
        let WireMessage {
            chat_external_id,
            reference,
            payload,
        } = msg;
        let cid = Self::chat_id(&chat_external_id)?;
        let reference = reference.as_ref();
        let reply_params = match reference {
            Some(reference) => Some(reply::build_reply(
                &reference.external_id,
                reference.excerpt.as_deref(),
            )?),
            None => None,
        };

        match payload {
            OutboundPayload::Text { content } => {
                let sent = self.send_text(cid, &content, reply_params).await?;
                receipt(&sent, reference, Vec::new())
            }
            OutboundPayload::Voice {
                bytes,
                content,
                prompt,
            } => {
                let sent = self
                    .send_voice(cid, bytes, content.as_deref(), reply_params)
                    .await?;
                receipt(
                    &sent,
                    reference,
                    vec![sent_attachment(
                        SentMedia::Voice,
                        &sent,
                        content.as_deref(),
                        Some(prompt),
                    )],
                )
            }
            OutboundPayload::Photo { source, content } => {
                let sent = self
                    .send_photo(cid, source, content.as_deref(), reply_params)
                    .await?;
                receipt(
                    &sent,
                    reference,
                    vec![sent_attachment(
                        SentMedia::Photo,
                        &sent,
                        content.as_deref(),
                        None,
                    )],
                )
            }
            OutboundPayload::Document {
                source,
                filename,
                content,
            } => {
                let sent = self
                    .send_document(cid, source, filename, content.as_deref(), reply_params)
                    .await?;
                receipt(
                    &sent,
                    reference,
                    vec![sent_attachment(
                        SentMedia::Document,
                        &sent,
                        content.as_deref(),
                        None,
                    )],
                )
            }
        }
    }

    fn start_typing(&self, chat_external_id: &str) -> Option<Box<dyn TypingGuard>> {
        let cid = match Self::chat_id(chat_external_id) {
            Ok(cid) => cid,
            Err(e) => {
                tracing::error!("Failed to send typing message: {e}");
                return None;
            }
        };
        Some(Box::new(TelegramTyping::start(self.bot.clone(), cid)))
    }

    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>> {
        self.files.download_file_cached(file_id).await
    }

    async fn file_url(&self, file_id: &str) -> Result<String> {
        self.srv.file_url(file_id).await
    }
}

fn input_file(source: OutboundSource) -> InputFile {
    match source {
        OutboundSource::Bytes(bytes) => InputFile::memory(bytes),
        OutboundSource::Url(url) => InputFile::url(url.into_url()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caption_is_cut_at_the_platform_ceiling() {
        let at_limit = "x".repeat(1024);
        assert_eq!(TelegramAdapter::truncate_caption(&at_limit), at_limit);

        let over = format!("{at_limit}tail");
        assert_eq!(TelegramAdapter::truncate_caption(&over), at_limit);

        // Cut by characters, never by bytes: a caption of one multi-byte character per unit survives
        // the cut whole.
        let multi_byte = "字".repeat(1025);
        let cut = TelegramAdapter::truncate_caption(&multi_byte);
        assert_eq!(cut.chars().count(), 1024);
        assert_eq!(cut, "字".repeat(1024));
    }
}
