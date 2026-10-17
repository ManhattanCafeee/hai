use std::fmt::Debug;

use async_trait::async_trait;

use super::ApiClient;
use crate::{agentcore::model_kind::ModelTarget, error::Result};

#[async_trait]
pub trait EmbeddingService: Debug + Send + Sync {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>>;
}

/// The `/v1/embeddings` transport: one text in, one vector out.
#[derive(Debug)]
pub struct EmbeddingApi {
    client: ApiClient,
    target: ModelTarget,
}

impl EmbeddingApi {
    pub fn new(target: ModelTarget) -> Result<Self> {
        Ok(Self {
            client: ApiClient::try_new()?,
            target,
        })
    }
}

#[async_trait]
impl EmbeddingService for EmbeddingApi {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        self.client.embed(&self.target, text).await
    }
}
