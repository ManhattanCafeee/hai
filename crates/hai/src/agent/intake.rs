//! The one inbound path of a channel: whatever a platform delivers becomes a stored change, an
//! answer the host composes, or nothing at all. A platform reads its own updates into an `Arrival`
//! and hands it here.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{
    agent::{
        channel::ChannelAccess,
        command::{CommandContext, Reply, execute, find},
        copy::HostCopy,
        runtime::registry::AgentRegistry,
        settings::Settings,
    },
    agentcore::Language,
    domain::{
        model::{Chat, ChatType, NewAttachment, Platform},
        repo::{ChangeKind, PlatformChange},
        service::Services,
        vo::EntryAudience,
    },
    error::{AppError, Result},
};

/// One message a platform read off its wire, in the shape the store and the agent speak.
pub(crate) struct Arrival {
    pub platform: Platform,
    pub chat: ArrivalChat,
    /// Absent for a message the platform reports without a sender (a channel post); the dispatcher
    /// drops those before they reach here.
    pub sender: Option<ArrivalSender>,
    pub text: String,
    pub attachments: Vec<NewAttachment>,
    pub external_id: String,
    pub reply_external_id: Option<String>,
    pub change: ChangeKind,
    pub sent_at: Option<jiff::Timestamp>,
    pub audience: EntryAudience,
    /// The platform's own message extras, stored as the row's meta.
    pub meta: serde_json::Value,
    pub text_kind: ArrivalText,
}

pub(crate) struct ArrivalChat {
    pub external_id: String,
    pub kind: ChatType,
    pub title: Option<String>,
}

pub(crate) struct ArrivalSender {
    pub external_id: String,
    pub meta: Option<serde_json::Value>,
}

/// What the platform made of the message's text.
pub(crate) enum ArrivalText {
    /// Ordinary prose: the platform found no command spelling in it.
    Ordinary,
    /// The platform's own greeting convention (Telegram's `/start`), answered and never stored.
    Greeting,
    /// A command of ours, with the payload the platform sliced off.
    Command { name: String, payload: String },
}

/// What an unauthorized message does: its greeting is answered once per cooldown, and nothing else
/// of its is. It is asked before anything is stored, and registers nothing — a chat outside the
/// allowlist is not ours to store.
pub(crate) fn answers_unauthorized(text_kind: &ArrivalText, answered_recently: bool) -> bool {
    matches!(text_kind, ArrivalText::Greeting) && !answered_recently
}

/// Unauthorized-chat replies: one answer per chat per cooldown, and a bounded tracking map.
struct UnauthorizedReplies {
    seen: Mutex<HashMap<String, Instant>>,
}

const UNAUTHORIZED_REPLY_COOLDOWN: Duration = Duration::from_secs(3600);
const UNAUTHORIZED_TRACKED_CHATS: usize = 1024;

impl UnauthorizedReplies {
    fn new() -> Self {
        Self {
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Whether this chat may be answered now.
    fn claim(&self, chat_external_id: &str) -> bool {
        let now = Instant::now();
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|_, at| now.duration_since(*at) < UNAUTHORIZED_REPLY_COOLDOWN);
        if seen.contains_key(chat_external_id) || seen.len() >= UNAUTHORIZED_TRACKED_CHATS {
            return false;
        }
        seen.insert(chat_external_id.to_owned(), now);
        true
    }
}

pub(crate) struct Intake {
    access: Arc<dyn ChannelAccess>,
    services: Services,
    settings: Settings,
    /// What host copy goes out in for a chat that has no row and no preference to read.
    default_language: Language,
    registry: AgentRegistry,
    unauthorized: UnauthorizedReplies,
}

impl Intake {
    pub(crate) fn new(
        access: Arc<dyn ChannelAccess>,
        services: Services,
        settings: Settings,
        default_language: Language,
        registry: AgentRegistry,
    ) -> Self {
        Self {
            access,
            services,
            settings,
            default_language,
            registry,
            unauthorized: UnauthorizedReplies::new(),
        }
    }

    pub(crate) async fn accept(&self, arrival: Arrival) -> Result<()> {
        if !self.access.admits(&arrival.chat.external_id) {
            tracing::warn!(
                "Unauthorized message attempt from chat_id: {}, user: {:?}",
                arrival.chat.external_id,
                arrival.sender.as_ref().map(|sender| &sender.external_id)
            );
            // The claim is spent on every unauthorized arrival, answered or not: one reply per chat
            // per hour is the whole budget.
            let answered_recently = !self.unauthorized.claim(&arrival.chat.external_id);
            if answers_unauthorized(&arrival.text_kind, answered_recently) {
                self.notice(&arrival, HostCopy::Unauthorized, self.default_language)
                    .await?;
            }
            return Ok(());
        }

        match &arrival.text_kind {
            ArrivalText::Greeting => {
                let chat = self.ensure_chat(&arrival).await?;
                let language = self.settings.language(chat.id).await?;
                self.notice(&arrival, HostCopy::Greeting, language).await
            }
            ArrivalText::Command { name, payload } => {
                self.run_command(&arrival, name, payload).await
            }
            ArrivalText::Ordinary => self.store(&arrival).await,
        }
    }

    /// Host copy is delivered, never stored: the conversation the model reads is the one people had
    /// with it, not the notices around it.
    async fn notice(&self, arrival: &Arrival, copy: HostCopy, language: Language) -> Result<()> {
        self.access
            .notice(&arrival.chat.external_id, &copy.render(language))
            .await
    }

    /// Registers the chat without its sender: a command carries no message, so no account is needed.
    async fn ensure_chat(&self, arrival: &Arrival) -> Result<Chat> {
        self.services
            .platform
            .get_or_create_chat(
                arrival.platform,
                &arrival.chat.external_id,
                arrival.chat.kind,
                arrival.chat.title.as_deref(),
                None,
            )
            .await
    }

    async fn run_command(&self, arrival: &Arrival, name: &str, payload: &str) -> Result<()> {
        // A slash message we own but do not know is ordinary conversation.
        let Some(name) = find(name) else {
            return self.store(arrival).await;
        };
        let chat = self.ensure_chat(arrival).await?;
        let ctx = CommandContext {
            chat_id: chat.id,
            settings: self.settings.clone(),
            agents: self.registry.clone(),
            services: self.services.clone(),
        };
        if let Reply::Said(said) = execute(&ctx, name.parse(payload)).await? {
            self.access
                .notice(&arrival.chat.external_id, &said.copy.render(said.language))
                .await?;
        }
        Ok(())
    }

    /// The conversation path: resolve the chat and its sender, then let one transaction write the
    /// message row and its accumulated entry before the agent is rung.
    async fn store(&self, arrival: &Arrival) -> Result<()> {
        let sender = arrival.sender.as_ref().ok_or_else(|| {
            AppError::message(format!(
                "arrival {} carries no sender to store",
                arrival.external_id
            ))
        })?;
        let (chat, account) = self
            .services
            .platform
            .ensure_chat_and_account(
                arrival.platform,
                &arrival.chat.external_id,
                arrival.chat.kind,
                arrival.chat.title.as_deref(),
                &sender.external_id,
                sender.meta.clone(),
            )
            .await?;
        self.services
            .platform
            .apply_platform_change(PlatformChange {
                chat_id: chat.id,
                account_id: account.id,
                text: arrival.text.clone(),
                attachments: arrival.attachments.clone(),
                external_id: arrival.external_id.clone(),
                meta: arrival.meta.clone(),
                sent_at: arrival.sent_at,
                kind: arrival.change,
                reply_external_id: arrival.reply_external_id.clone(),
                audience: arrival.audience,
            })
            .await?;
        self.registry.get_or_create(chat.id).await.notify();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::ChatId;

    #[test]
    fn only_an_unauthorized_greeting_inside_its_cooldown_is_answered() {
        assert!(answers_unauthorized(&ArrivalText::Greeting, false));
        assert!(
            !answers_unauthorized(&ArrivalText::Greeting, true),
            "one answer per hour"
        );

        let command = ArrivalText::Command {
            name: "status".to_owned(),
            payload: String::new(),
        };
        for answered_recently in [false, true] {
            assert!(!answers_unauthorized(
                &ArrivalText::Ordinary,
                answered_recently
            ));
            assert!(!answers_unauthorized(&command, answered_recently));
        }
    }

    #[test]
    fn one_reply_per_chat_per_cooldown() {
        let replies = UnauthorizedReplies::new();
        assert!(replies.claim("7"));
        assert!(!replies.claim("7"), "the same chat is not answered twice");
        assert!(replies.claim("8"), "another chat has its own budget");
    }

    #[test]
    fn the_tracking_map_admits_up_to_its_cap() {
        let replies = UnauthorizedReplies::new();
        let admitted: Vec<bool> = (0..UNAUTHORIZED_TRACKED_CHATS)
            .map(|chat| replies.claim(&ChatId(chat as i64 + 1).to_string()))
            .collect();

        assert!(
            admitted.iter().all(|admitted| *admitted),
            "every chat up to the cap is admitted once"
        );
        assert!(
            !replies.claim(&ChatId(UNAUTHORIZED_TRACKED_CHATS as i64 + 1).to_string()),
            "the chat past the cap is refused"
        );
        assert!(!replies.claim("1"), "a tracked chat is not answered twice");
    }
}
