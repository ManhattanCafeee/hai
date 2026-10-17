use std::sync::Arc;

use derive_more::Deref;

use crate::{
    agent::{multimodal::MultimodalService, runtime::event::AgentEventBus, settings::Settings},
    agentcore::{
        api::{
            embedding::EmbeddingApi,
            rerank::{RerankApi, Scorer},
        },
        provider::ChatTransport,
    },
    config::{AppConfig, ProviderRegistry, model_role},
    domain::{
        db,
        repo::{Repos, vector},
        service::Services,
    },
    error::Result,
};

#[derive(Clone, Deref)]
pub struct AppContext {
    inner: Arc<AppContextInner>,
}

pub struct AppContextInner {
    pub cfg: Arc<AppConfig>,
    pub settings: Settings,
    pub services: Services,
    pub chat_transport: ChatTransport,
    pub multimodal: MultimodalService,
    pub event_bus: AgentEventBus,
}

/// Everything a process builds before it can serve: the provider registry, the ModelRoles resolved
/// against it, and the database services they feed.
pub(crate) struct Assembly {
    pub chat_transport: ChatTransport,
    pub multimodal: MultimodalService,
    pub services: Services,
    pub repos: Repos,
}

/// Wires a process in the one order it requires: providers, then the model roles they resolve, then
/// the pool.
pub(crate) async fn assemble(cfg: &AppConfig) -> Result<Assembly> {
    let providers = ProviderRegistry::discover(cfg).await;

    let roles = model_role::resolve(cfg, &providers)?;
    let multimodal = MultimodalService::from_config(cfg, &roles)?;
    let retrieval = &cfg.agent.retrieval;
    let scorer = roles
        .rerank
        .as_ref()
        .map(|target| -> Result<Scorer> {
            Ok(Scorer::new(
                Arc::new(RerankApi::new(target.clone(), retrieval.timeout_secs)?),
                retrieval.candidates,
                retrieval.candidate_chars,
            ))
        })
        .transpose()?;
    let chat_transport = roles.chat;

    let (pool, dimension) = db::init_db_with_dimension(cfg).await?;
    vector::assert_embedding_dimension(&pool, dimension).await?;
    let repos = Repos::new(pool);
    let services = Services::new(
        repos.clone(),
        Arc::new(EmbeddingApi::new(roles.embedding)?),
        scorer,
    );

    Ok(Assembly {
        chat_transport,
        multimodal,
        services,
        repos,
    })
}

impl AppContext {
    pub async fn new(cfg: Arc<AppConfig>) -> Result<Self> {
        let Assembly {
            chat_transport,
            multimodal,
            services,
            repos,
        } = assemble(&cfg).await?;
        let settings = Settings::new(Arc::clone(&cfg), services.clone());
        let event_bus = AgentEventBus::new(repos.event.clone());

        Ok(Self {
            inner: Arc::new(AppContextInner {
                cfg,
                settings,
                services,
                chat_transport,
                multimodal,
                event_bus,
            }),
        })
    }
}
