use std::collections::{HashMap, HashSet};

use derive_more::Deref;

use crate::{
    agent::{
        channel::ChannelProfile,
        context::{ContextMessages, fmt::display_name},
    },
    domain::{
        model::{Account, Attachment, Chat, Memory, Message, Perception, Topic},
        vo::{AccountId, AttachmentId, MessageId},
    },
};

/// One run's full context snapshot: the inputs plus the attachment and perception indices rendering reads.
#[derive(Deref)]
pub struct RenderContext {
    #[deref]
    pub data: RenderContextData,

    accounts_by_id: HashMap<AccountId, usize>,
}

/// The platform-independent input `RenderContext` is built from.
#[derive(Debug)]
pub struct RenderContextData {
    pub channel: ChannelProfile,
    pub chat: Option<Chat>,
    pub current_time: String,
    /// The messages this turn's batch changed: they render below the `new messages` divider.
    pub new_message_ids: HashSet<MessageId>,
    pub first_turn: bool,

    pub messages: ContextMessages,

    /// The attachments of the window and of the out-of-window reply targets, by message.
    pub attachments_by_message: HashMap<MessageId, Vec<Attachment>>,
    pub perception_by_attachment: HashMap<AttachmentId, Vec<Perception>>,
    pub same_resource_as: HashMap<AttachmentId, AttachmentId>,

    pub accounts: Vec<Account>,

    pub current_topics: Vec<Topic>,
    pub pinned_memories: Vec<Memory>,
    pub recalled_memories: Vec<Memory>,
    pub recalled_topics: Vec<Topic>,

    pub perceptions: Vec<Perception>,

    /// Topic idle timeout in hours; a topic idle past it renders as need-close.
    pub topic_idle_hours: i64,
}

impl RenderContext {
    pub fn new(data: RenderContextData) -> Self {
        let mut ctx = Self {
            accounts_by_id: HashMap::new(),
            data,
        };
        ctx.build_indices();
        ctx
    }

    fn build_indices(&mut self) {
        for (i, account) in self.data.accounts.iter().enumerate() {
            self.accounts_by_id.insert(account.id, i);
        }
    }

    pub fn get_account(&self, id: AccountId) -> Option<&Account> {
        self.accounts_by_id
            .get(&id)
            .copied()
            .and_then(|i| self.data.accounts.get(i))
    }

    pub fn sender_name(&self, msg: &Message) -> String {
        let aid = msg.account_id.unwrap_or_default();

        if aid == self.channel.account_id {
            let n = &self.channel;
            return if n.username.is_empty() {
                n.name.clone()
            } else {
                format!("{} (@{})", n.name, n.username)
            };
        }

        display_name(self.get_account(aid), aid)
    }

    pub fn perceptions(&self) -> &[Perception] {
        &self.data.perceptions
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::RenderContextData;
    use crate::{
        agent::{channel::ChannelProfile, context::ContextMessages},
        domain::{
            model::{Chat, ChatType, Platform},
            vo::{AccountId, ChatId},
        },
    };

    /// The quiet `RenderContextData` the section tests start from: a channel and a group chat, no
    /// messages, accounts, topics or memories. A test overrides only the fields it asserts on.
    pub(crate) fn render_context_data() -> RenderContextData {
        let now = jiff::Timestamp::now();
        RenderContextData {
            channel: ChannelProfile {
                account_id: AccountId(1),
                username: "channel".into(),
                name: "Bot".into(),
            },
            chat: Some(Chat {
                id: ChatId(2),
                platform: Platform::Telegram,
                external_id: "2".into(),
                chat_type: ChatType::Group,
                name: None,
                meta: None,
                created_at: now,
                updated_at: now,
            }),
            current_time: String::new(),
            new_message_ids: Default::default(),
            first_turn: true,
            messages: ContextMessages::new(Vec::new(), Default::default()),
            attachments_by_message: Default::default(),
            perception_by_attachment: Default::default(),
            same_resource_as: Default::default(),
            accounts: Vec::new(),
            current_topics: Vec::new(),
            pinned_memories: Vec::new(),
            recalled_memories: Vec::new(),
            recalled_topics: Vec::new(),
            perceptions: Vec::new(),
            topic_idle_hours: 2,
        }
    }
}
