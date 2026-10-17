use std::fmt;

use teloxide::{
    Bot,
    net::Download,
    payloads::SendMessageSetters,
    prelude::Requester,
    types::{ChatId, FileId, ParseMode, ReplyParameters},
};

use super::markdown::markdown_v2;
use crate::{
    config::Paths,
    domain::model::Platform,
    error::{AppError, Result, bail},
    util::cache::FileCache,
};

#[derive(Clone)]
pub struct TelegramService {
    bot: Bot,
    http: reqwest::Client,
}

/// The Telegram side of file access: the bytes and the URL of one file handle, the download cached
/// under the platform's own key.
pub struct TelegramFiles {
    srv: TelegramService,
    cache: FileCache,
}

impl TelegramFiles {
    pub fn new(srv: TelegramService) -> Self {
        Self {
            srv,
            cache: FileCache::new(Paths::inferred().file_cache_dir().to_owned()),
        }
    }

    pub(crate) async fn download_file_cached(&self, file_id: &str) -> Result<Vec<u8>> {
        let cache_key = format!("{}-{}", Platform::Telegram.as_str(), file_id);
        if let Some(cached) = self.cache.find(&cache_key).await {
            return Ok(cached);
        }
        let data = self.srv.download(file_id).await?;
        // The bytes are in hand either way: a cache that cannot be written costs the next read a
        // download, never this one its result.
        if let Err(e) = self.cache.add(&cache_key, &data).await {
            tracing::warn!(file_id, error = %e, "Failed to cache a downloaded file");
        }
        Ok(data)
    }
}

/// The Telegram wire's own error mapping: the vendor type stops here rather than in the shared error
/// enum.
pub(super) fn telegram_err(e: teloxide::RequestError) -> AppError {
    AppError::adapter("telegram", e)
}

impl TelegramService {
    pub fn new(bot: Bot) -> Self {
        Self {
            bot,
            http: reqwest::Client::new(),
        }
    }

    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        let file = self
            .bot
            .get_file(FileId(file_id.to_string()))
            .await
            .map_err(telegram_err)?;
        let mut data = Vec::new();
        self.bot
            .download_file(&file.path, &mut data)
            .await
            .map_err(|e| AppError::adapter("telegram", e))?;
        Ok(data)
    }

    pub async fn file_url(&self, file_id: &str) -> Result<String> {
        let file = self
            .bot
            .get_file(FileId(file_id.to_string()))
            .await
            .map_err(telegram_err)?;
        Ok(format!(
            "https://api.telegram.org/file/bot{}/{}",
            self.bot.token(),
            file.path
        ))
    }

    pub async fn send_rich_message(
        &self,
        chat_id: ChatId,
        markdown: &str,
        reply_params: Option<&ReplyParameters>,
    ) -> Result<teloxide::types::Message> {
        let mut payload = serde_json::json!({
            "chat_id": chat_id.0,
            "rich_message": { "markdown": markdown },
        });
        if let Some(params) = reply_params {
            payload["reply_parameters"] = serde_json::to_value(params)?;
        }

        let url = format!(
            "https://api.telegram.org/bot{}/sendRichMessage",
            self.bot.token()
        );
        let resp: serde_json::Value = self
            .http
            .post(url)
            .json(&payload)
            .send()
            .await?
            .json()
            .await?;

        if resp["ok"].as_bool() != Some(true) {
            let desc = resp["description"].as_str().unwrap_or("unknown error");
            bail!(AppError::message(format!("sendRichMessage failed: {desc}")));
        }

        let msg: teloxide::types::Message = serde_json::from_value(resp["result"].clone())
            .map_err(|e| AppError::message(format!("sendRichMessage response parse: {e}")))?;
        Ok(msg)
    }
}

/// Sends `content` as MarkdownV2 and retries it as plain text when Telegram refuses the parse — the
/// text itself is what must arrive, formatting is best effort.
pub(super) async fn send_markdown_with_fallback(
    bot: &Bot,
    chat_id: ChatId,
    content: &str,
    reply_params: Option<&ReplyParameters>,
) -> Result<teloxide::types::Message> {
    let markdown = markdown_v2(content);
    let send_md = bot.send_message(chat_id, &markdown);
    let send_md = match reply_params {
        Some(p) => send_md.reply_parameters(p.clone()),
        None => send_md,
    };
    match send_md.parse_mode(ParseMode::MarkdownV2).await {
        Ok(msg) => Ok(msg),
        Err(e) => {
            tracing::warn!(chat_id = chat_id.0, error = %e, "markdown send failed; retrying as plain text");
            let send_plain = bot.send_message(chat_id, content);
            let send_plain = match reply_params {
                Some(p) => send_plain.reply_parameters(p.clone()),
                None => send_plain,
            };
            send_plain.await.map_err(telegram_err)
        }
    }
}

impl fmt::Debug for TelegramService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelegramService").finish_non_exhaustive()
    }
}
