use std::{fmt::Debug, sync::Arc, time::Duration};

use async_trait::async_trait;

use super::ApiClient;
use crate::{agentcore::model_kind::ModelTarget, error::Result, util::text::truncate_chars};

/// Scores candidates against a query. A score only orders the candidates of the request that
/// produced it: it is not a probability, and another request's scores are not comparable to these.
#[async_trait]
pub trait RerankService: Debug + Send + Sync {
    /// One score per candidate, in the caller's order; higher is more relevant.
    async fn rerank(&self, query: &str, candidates: &[&str]) -> Result<Vec<f64>>;
}

/// The rerank ModelRole as the domain consumes it: the transport plus the budgets every call
/// shares.
#[derive(Debug, Clone)]
pub struct Scorer {
    service: Arc<dyn RerankService>,
    candidates: usize,
    candidate_chars: usize,
}

impl Scorer {
    pub fn new(service: Arc<dyn RerankService>, candidates: usize, candidate_chars: usize) -> Self {
        Self {
            service,
            candidates,
            candidate_chars,
        }
    }

    /// How many fused candidates one rerank request sees for a caller keeping `limit` results.
    pub fn pool_size(&self, limit: i64) -> usize {
        ((limit * 4).max(10) as usize).min(self.candidates)
    }

    /// Scores `candidates` against `query`, clipping each one to the configured budget. An empty
    /// candidate set is answered without a request: there is nothing to score.
    pub async fn rank(&self, query: &str, candidates: &[&str]) -> Result<Vec<f64>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let clipped: Vec<String> = candidates
            .iter()
            .map(|candidate| truncate_chars(candidate, self.candidate_chars))
            .collect();
        let clipped: Vec<&str> = clipped.iter().map(String::as_str).collect();
        self.service.rerank(query, &clipped).await
    }
}

/// The `/v1/rerank` transport (Cohere, Voyage, Jina, TEI, llama.cpp, OpenRouter): documents in,
/// relevance scores out.
#[derive(Debug)]
pub struct RerankApi {
    client: ApiClient,
    target: ModelTarget,
    timeout: Duration,
}

impl RerankApi {
    pub fn new(target: ModelTarget, timeout_secs: u64) -> Result<Self> {
        Ok(Self {
            client: ApiClient::try_new()?,
            target,
            timeout: Duration::from_secs(timeout_secs),
        })
    }
}

#[async_trait]
impl RerankService for RerankApi {
    async fn rerank(&self, query: &str, candidates: &[&str]) -> Result<Vec<f64>> {
        self.client
            .rerank(&self.target, query, candidates, self.timeout)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::FakeRerank;

    fn scorer(candidates: usize) -> Scorer {
        Scorer::new(Arc::new(FakeRerank::zeros()), candidates, 500)
    }

    #[test]
    fn the_pool_grows_with_the_keep_count_and_stops_at_the_budget() {
        let scorer = scorer(50);

        assert_eq!(scorer.pool_size(1), 10);
        assert_eq!(scorer.pool_size(5), 20);
        assert_eq!(scorer.pool_size(8), 32);
        assert_eq!(scorer.pool_size(100), 50);
    }

    #[tokio::test]
    async fn an_empty_candidate_set_makes_no_request() {
        let service = Arc::new(FakeRerank::zeros());
        let scorer = Scorer::new(service.clone(), 30, 768);

        assert!(scorer.rank("any query", &[]).await.unwrap().is_empty());
        assert_eq!(service.calls(), 0);
    }
}
