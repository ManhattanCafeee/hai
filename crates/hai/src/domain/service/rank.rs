//! Ranking policy: the candidate pool a rerank request carries, and how its scores reorder what
//! retrieval fused. The transport and its budgets live in `agentcore::api::rerank`.

use std::borrow::Cow;

use crate::{
    agentcore::api::rerank::Scorer,
    error::{AppError, Result, bail},
};

/// What a failed rerank call means for one caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScorerErrorPolicy {
    Fail,
    KeepFusionOrder,
}

/// The candidate pool for a caller keeping `limit` results: a scorer narrows it to what one request
/// carries, and without one the keep count is the pool.
pub(crate) fn pool(scorer: Option<&Scorer>, limit: i64) -> usize {
    scorer.map_or(limit as usize, |scorer| scorer.pool_size(limit))
}

/// Reorders the fused candidates by one rerank request, logging its scores under `label`.
///
/// `text_of` extracts what each candidate is scored by, so a caller that already holds the text
/// hands it over instead of copying it. A failed call follows `on_error` — `KeepFusionOrder`
/// returns the candidates untouched, fusion scores included — while a score count that does not
/// match the candidates is always an error, whichever policy is in force.
pub(crate) async fn rerank<T>(
    scorer: &Scorer,
    query: &str,
    candidates: Vec<(T, f64)>,
    text_of: impl Fn(&T) -> Cow<'_, str>,
    on_error: ScorerErrorPolicy,
    label: &str,
) -> Result<Vec<(T, f64)>> {
    let scored = {
        let texts: Vec<Cow<'_, str>> = candidates.iter().map(|(item, _)| text_of(item)).collect();
        let texts: Vec<&str> = texts.iter().map(|text| text.as_ref()).collect();
        scorer.rank(query, &texts).await
    };
    let scores = match scored {
        Ok(scores) => scores,
        Err(e) if on_error == ScorerErrorPolicy::KeepFusionOrder => {
            tracing::warn!(error = %e, "rerank failed; keeping the fusion order");
            return Ok(candidates);
        }
        Err(e) => return Err(e),
    };
    tracing::debug!(candidates = candidates.len(), scores = ?scores, "{label} rerank");

    apply_scores(
        candidates.into_iter().map(|(item, _)| item).collect(),
        scores,
    )
}

/// Applies one set of scores to the items they scored: the scores are paired in the items' order
/// and the result comes back best first. A call whose score count differs fails rather than
/// mis-ranking.
fn apply_scores<T>(items: Vec<T>, scores: Vec<f64>) -> Result<Vec<(T, f64)>> {
    if scores.len() != items.len() {
        bail!(AppError::message(format!(
            "rerank returned {} scores for {} candidates",
            scores.len(),
            items.len()
        )));
    }
    let mut ordered: Vec<(T, f64)> = items.into_iter().zip(scores).collect();
    ordered.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(ordered)
}
