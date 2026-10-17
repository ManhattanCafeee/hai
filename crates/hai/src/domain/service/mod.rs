pub mod chunking;
pub mod conversation_record;
pub(crate) mod fuse;
pub(crate) mod hydrate;
pub mod identity;
pub mod knowledge;
pub mod memory;
pub mod message;
pub mod perception;
pub mod platform;
pub mod preference;
pub(crate) mod rank;
pub mod recall;
pub mod scheduled_task;
pub mod topic;

use std::sync::Arc;

pub use conversation_record::ConversationRecordService;
use derive_more::Deref;
pub use identity::IdentityService;
pub use knowledge::{KnowledgeService, RelatedChunk};
pub use memory::{MemoryService, RecordOutcome};
pub use message::{MessageService, NewAgentMessage};
pub use perception::PerceptionService;
pub use platform::PlatformService;
pub use preference::PreferenceService;
pub(crate) use rank::ScorerErrorPolicy;
pub use recall::{RecallItem, RecallQuery, RecallScope, recall};
pub use scheduled_task::ScheduledTaskService;
pub use topic::TopicService;

use crate::{
    agentcore::api::{embedding::EmbeddingService, rerank::Scorer},
    domain::repo::Repos,
};

#[derive(Debug, Clone, Deref)]
pub struct Services(Arc<ServicesInner>);

#[derive(Debug)]
pub struct ServicesInner {
    pub platform: PlatformService,
    pub identity: IdentityService,
    pub topic: TopicService,
    pub message: MessageService,
    pub memory: MemoryService,
    pub knowledge: KnowledgeService,
    pub conversation: ConversationRecordService,
    pub scheduled_task: ScheduledTaskService,
    pub embedding: Arc<dyn EmbeddingService>,
    /// Absent when `[model-roles.rerank]` is not configured; retrieval then keeps the fusion order.
    pub scorer: Option<Scorer>,
    pub perception: PerceptionService,
    pub preference: PreferenceService,
}

impl Services {
    pub fn new(repos: Repos, embedding: Arc<dyn EmbeddingService>, scorer: Option<Scorer>) -> Self {
        let platform = PlatformService::new(repos.clone());
        let identity = IdentityService::new(repos.clone());
        let message = MessageService::new(repos.clone());
        let conversation = ConversationRecordService::new(repos.clone());
        let scheduled_task = ScheduledTaskService::new(repos.clone());
        let topic = TopicService::new(repos.clone(), Arc::clone(&embedding));
        let memory = MemoryService::new(repos.clone(), Arc::clone(&embedding));
        let perception = PerceptionService::new(repos.clone(), Arc::clone(&embedding));
        let knowledge =
            KnowledgeService::new(repos.clone(), Arc::clone(&embedding), scorer.clone());
        let preference = PreferenceService::new(repos.clone());

        Self(Arc::new(ServicesInner {
            platform,
            identity,
            topic,
            message,
            memory,
            knowledge,
            conversation,
            scheduled_task,
            embedding,
            perception,
            preference,
            scorer,
        }))
    }
}
