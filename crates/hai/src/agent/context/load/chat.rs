use std::collections::{HashMap, HashSet};

use crate::{
    domain::{
        model::{Account, Message},
        service::Services,
        vo::{AccountId, ChatId, MessageId},
    },
    error::{AppError, Result},
};

pub async fn load_chat(services: &Services, chat_id: ChatId) -> Result<crate::domain::model::Chat> {
    services
        .platform
        .get_chat_by_id(chat_id)
        .await?
        .ok_or_else(|| AppError::message(format!("Chat not found: {chat_id}")))
}

pub async fn load_reply_map(
    services: &Services,
    messages: &[Message],
) -> Result<HashMap<MessageId, Message>> {
    let window: HashSet<MessageId> = messages.iter().map(|m| m.id).collect();
    let missing: Vec<MessageId> = messages
        .iter()
        .filter_map(|m| m.reply_to_id)
        .filter(|rid| !window.contains(rid))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    if missing.is_empty() {
        return Ok(HashMap::new());
    }
    let found = services.message.get_messages_by_ids(&missing).await?;
    Ok(found.into_iter().map(|m| (m.id, m)).collect())
}

/// The accounts plus their identity siblings; reference sender names depend on them, and a missing
/// account degrades to `User{id}`. Sorted by id, because the rendered prompt must not depend on the
/// order a hash map happened to produce.
pub async fn collect_accounts(
    services: &Services,
    messages: &[Message],
    reply_map: &HashMap<MessageId, Message>,
) -> Result<Vec<Account>> {
    let raw_ids: HashSet<AccountId> = messages
        .iter()
        .chain(reply_map.values())
        .filter_map(|m| m.account_id)
        .collect();
    let mut account_map: HashMap<AccountId, Account> = HashMap::new();

    for id in raw_ids {
        if account_map.contains_key(&id) {
            continue;
        }
        if let Some(account) = services.platform.get_account_by_id(id).await? {
            if let Some(identity_id) = account.identity_id {
                for sibling in services.platform.get_identity_accounts(identity_id).await? {
                    account_map.insert(sibling.id, sibling);
                }
            } else {
                account_map.insert(id, account);
            }
        }
    }
    let mut accounts: Vec<Account> = account_map.into_values().collect();
    accounts.sort_by_key(|account| account.id);
    Ok(accounts)
}
