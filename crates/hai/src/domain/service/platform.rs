use crate::{
    domain::{
        model::{Account, Chat, ChatType, Message, NewChat, Platform},
        repo::{PlatformChange, Repos},
        vo::{AccountId, ChannelKey, ChatId, IdentityId},
    },
    error::Result,
};

#[derive(Debug)]
pub struct PlatformService {
    repos: Repos,
}

impl PlatformService {
    pub fn new(repos: Repos) -> Self {
        Self { repos }
    }

    /// One incoming platform change: the message row and its `ChatEntry` land together.
    pub async fn apply_platform_change(&self, change: PlatformChange) -> Result<Message> {
        self.repos.ingest.apply(change).await
    }

    pub async fn ensure_chat_and_account(
        &self,
        platform: Platform,
        external_chat_id: &str,
        chat_type: ChatType,
        chat_name: Option<&str>,
        external_user_id: &str,
        user_meta: Option<serde_json::Value>,
    ) -> Result<(Chat, Account)> {
        let chat = self
            .get_or_create_chat(platform, external_chat_id, chat_type, chat_name, None)
            .await?;
        let account = self
            .get_or_create_account(platform, external_user_id, user_meta)
            .await?;
        Ok((chat, account))
    }

    async fn get_or_create_account(
        &self,
        platform: Platform,
        external_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Result<Account> {
        if let Some(account) = self
            .repos
            .account
            .by_platform_external_id(platform, external_id)
            .await?
        {
            // Best-effort: a failed refresh must not fail the message path.
            if let Err(e) = self
                .repos
                .account
                .update_last_active_at(account.id, jiff::Timestamp::now())
                .await
            {
                tracing::warn!(?e, "failed to refresh account last_active_at");
            }
            return Ok(account);
        }
        let now = jiff::Timestamp::now();
        self.repos
            .account
            .create(platform, external_id, meta, now, now, now)
            .await
    }

    pub async fn get_or_create_chat(
        &self,
        platform: Platform,
        external_id: &str,
        chat_type: ChatType,
        name: Option<&str>,
        meta: Option<serde_json::Value>,
    ) -> Result<Chat> {
        if let Some(chat) = self
            .repos
            .chat
            .by_platform_external_id(platform, external_id)
            .await?
        {
            return Ok(chat);
        }
        let now = jiff::Timestamp::now();
        self.repos
            .chat
            .create(NewChat {
                platform,
                external_id: external_id.to_owned(),
                chat_type,
                name: name.map(str::to_owned),
                meta,
                created_at: now,
                updated_at: now,
            })
            .await
    }

    pub async fn get_chat_by_id(&self, id: ChatId) -> Result<Option<Chat>> {
        self.repos.chat.by_id(id).await
    }

    pub async fn get_account_by_id(&self, id: AccountId) -> Result<Option<Account>> {
        self.repos.account.by_id(id).await
    }

    pub async fn get_identity_accounts(&self, identity_id: IdentityId) -> Result<Vec<Account>> {
        self.repos.account.by_identity_id(identity_id).await
    }

    pub async fn ensure_bot_account(
        &self,
        platform: Platform,
        key: &ChannelKey,
    ) -> Result<Account> {
        // The account a channel speaks through, keyed `bot:<channel key>`: the prefix keeps a second
        // bot on the same platform from colliding with the first, and the spelling is what the
        // deploy's account rows must carry.
        let external_id = format!("bot:{key}");
        // A database written before the bot account carried its key holds one shared `(system, bot)`
        // row; adopting it keeps the history that references its id (the `own` marking in the
        // render) pointing at the account this build sends through.
        if let Some(account) = self
            .repos
            .account
            .adopt_legacy_bot_account(platform, &external_id)
            .await?
        {
            return Ok(account);
        }
        self.get_or_create_account(platform, &external_id, None)
            .await
    }
}
