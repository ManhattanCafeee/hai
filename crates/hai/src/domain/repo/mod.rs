pub mod account;
pub mod attachment;
pub mod chat;
pub mod chat_preference;
pub mod conversation_record;
pub mod event;
pub mod identity;
pub mod ingest;
pub mod knowledge_chunk;
pub mod knowledge_document;
pub(crate) mod lexical;
pub mod memory;
pub mod message;
pub mod perception;
pub mod scheduled_task;
pub mod topic;
pub mod vector;

pub use account::AccountRepo;
pub use attachment::AttachmentRepo;
pub use chat::ChatRepo;
pub use chat_preference::ChatPreferenceRepo;
pub use conversation_record::ConversationRecordRepo;
pub use event::EventRepo;
pub use identity::IdentityRepo;
pub use ingest::{ChangeKind, IngestRepo, PlatformChange};
pub use knowledge_chunk::KnowledgeChunkRepo;
pub use knowledge_document::KnowledgeDocumentRepo;
pub use memory::MemoryRepo;
pub use message::MessageRepo;
pub use perception::PerceptionRepo;
pub use scheduled_task::ScheduledTaskRepo;
use sqlx::PgPool;
pub use topic::TopicRepo;

#[derive(Debug, Clone)]
pub struct Repos {
    pub account: AccountRepo,
    pub attachment: AttachmentRepo,
    pub chat: ChatRepo,
    pub chat_preference: ChatPreferenceRepo,
    pub conversation_record: ConversationRecordRepo,
    pub event: EventRepo,
    pub identity: IdentityRepo,
    pub ingest: IngestRepo,
    pub knowledge_chunk: KnowledgeChunkRepo,
    pub knowledge_document: KnowledgeDocumentRepo,
    pub memory: MemoryRepo,
    pub message: MessageRepo,
    pub perception: PerceptionRepo,
    pub scheduled_task: ScheduledTaskRepo,
    pub topic: TopicRepo,
}

impl Repos {
    pub fn new(pool: PgPool) -> Self {
        Self {
            account: AccountRepo::new(pool.clone()),
            attachment: AttachmentRepo::new(pool.clone()),
            chat: ChatRepo::new(pool.clone()),
            chat_preference: ChatPreferenceRepo::new(pool.clone()),
            conversation_record: ConversationRecordRepo::new(pool.clone()),
            event: EventRepo::new(pool.clone()),
            identity: IdentityRepo::new(pool.clone()),
            ingest: IngestRepo::new(pool.clone()),
            knowledge_chunk: KnowledgeChunkRepo::new(pool.clone()),
            knowledge_document: KnowledgeDocumentRepo::new(pool.clone()),
            memory: MemoryRepo::new(pool.clone()),
            message: MessageRepo::new(pool.clone()),
            perception: PerceptionRepo::new(pool.clone()),
            scheduled_task: ScheduledTaskRepo::new(pool.clone()),
            topic: TopicRepo::new(pool.clone()),
        }
    }
}
