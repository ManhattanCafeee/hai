use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fmt::Write as _,
    sync::Arc,
};

use futures::{TryStreamExt, stream::StreamExt};
use sha2::{Digest, Sha256};

use super::{
    chunking::{self, ChunkCfg},
    fuse::reciprocal_rank_fusion,
    hydrate,
    rank::{self, ScorerErrorPolicy},
};
use crate::{
    agentcore::api::{embedding::EmbeddingService, rerank::Scorer},
    domain::{
        model::{KnowledgeChunk, KnowledgeDocument, KnowledgeDocumentMeta},
        repo::{
            Repos,
            lexical::{MAX_LEXICAL_TERMS, lexical_terms},
        },
        vo::{Collection, KnowledgeChunkId, KnowledgeDocumentId},
    },
    error::{AppError, Result, bail},
};

/// Chunker algorithm version; bump it whenever the algorithm or its parameters change, which is
/// what makes `reindex` rebuild the stored chunks.
pub const CHUNKER_VERSION: &str = "v1";

/// A document that would yield more chunks than this is rejected at import, not truncated.
pub const MAX_CHUNKS_PER_DOC: usize = 10_000;

const MAX_CONCURRENT_EMBEDDINGS: usize = 10;

#[derive(Debug)]
pub struct KnowledgeService {
    repos: Repos,
    embedding: Arc<dyn EmbeddingService>,
    scorer: Option<Scorer>,
}

#[derive(Debug)]
pub struct RelatedChunk {
    pub chunk_id: KnowledgeChunkId,
    pub document_id: KnowledgeDocumentId,
    pub content: String,
    pub document_title: String,
    pub collection: Collection,
    /// L2 distance from the vector leg; absent when the chunk came from the lexical leg alone.
    pub distance: Option<f64>,
    /// The reranker's score, when reranking is configured; comparable only inside one request.
    pub score: Option<f64>,
}

#[derive(Debug)]
pub struct ImportOutcome {
    /// `false` = content unchanged (same source, same hash), nothing was written.
    pub imported: bool,
    pub chunk_count: usize,
}

impl KnowledgeService {
    pub fn new(repos: Repos, embedding: Arc<dyn EmbeddingService>, scorer: Option<Scorer>) -> Self {
        Self {
            repos,
            embedding,
            scorer,
        }
    }

    /// Idempotent import: a source whose content hash is unchanged is skipped, anything else
    /// replaces the document and its chunks wholesale.
    ///
    /// `force` skips the hash check so a stale chunker version is rebuilt even though the content
    /// did not change, which is also what refreshes `meta.chunker_version`.
    ///
    /// Embeddings are generated before the write; document, chunks and vectors then land in one
    /// transaction, so a failure leaves no orphans and a rerun is safe.
    pub async fn upsert_document(
        &self,
        source: &str,
        title: &str,
        collection: &Collection,
        content: &str,
        cfg: &ChunkCfg,
        force: bool,
    ) -> Result<ImportOutcome> {
        let file_hash = sha256_hex(content);

        let existing = self.repos.knowledge_document.find_by_source(source).await?;

        if !force
            && let Some(doc) = &existing
            && parse_doc_meta(doc).is_some_and(|m| m.file_hash == file_hash)
        {
            return Ok(ImportOutcome {
                imported: false,
                chunk_count: 0,
            });
        }

        let chunks = chunking::chunk(content, cfg);
        if chunks.len() > MAX_CHUNKS_PER_DOC {
            bail!(AppError::message(format!(
                "document '{source}' yields {} chunks, exceeding limit {MAX_CHUNKS_PER_DOC}",
                chunks.len()
            )));
        }

        let embeddings: Vec<Vec<f32>> = futures::stream::iter(chunks.iter().cloned())
            .map(|chunk_text| async move { self.embedding.generate_embedding(&chunk_text).await })
            .buffered(MAX_CONCURRENT_EMBEDDINGS)
            .try_collect()
            .await?;

        let meta_json = serde_json::to_value(KnowledgeDocumentMeta {
            file_hash,
            chunker_version: CHUNKER_VERSION.to_string(),
        })?;
        let doc = KnowledgeDocument::new(
            title.to_string(),
            collection.clone(),
            source.to_string(),
            content.to_string(),
            Some(meta_json),
        );
        let chunk_rows: Vec<KnowledgeChunk> = chunks
            .iter()
            .enumerate()
            .map(|(seq, chunk_text)| KnowledgeChunk::new(doc.id, seq as i32, chunk_text.clone()))
            .collect();
        self.repos
            .knowledge_document
            .upsert_document(source, &doc, &chunk_rows, &embeddings)
            .await?;

        Ok(ImportOutcome {
            imported: true,
            chunk_count: chunks.len(),
        })
    }

    pub async fn delete(&self, document_id: KnowledgeDocumentId) -> Result<()> {
        self.repos.knowledge_document.delete(document_id).await
    }

    pub async fn list(&self, collection: Option<&Collection>) -> Result<Vec<KnowledgeDocument>> {
        self.repos.knowledge_document.list(collection).await
    }

    /// Hybrid search over chunks, hydrated with document provenance: a vector leg and a lexical leg
    /// are fused by reciprocal rank, then the scorer reorders the fused candidates. `limit` is how
    /// many chunks survive; the candidate pool is larger while a scorer is configured. A non-empty
    /// `collections` narrows the search to those collections; empty searches every collection.
    pub async fn search(
        &self,
        query: &str,
        limit: i64,
        collections: &[Collection],
    ) -> Result<Vec<RelatedChunk>> {
        // Guard: a negative LIMIT is undefined in PostgreSQL, and an oversized one would let a
        // caller pull far more than retrieval can use.
        let limit = limit.clamp(1, 1000);
        let pool = rank::pool(self.scorer.as_ref(), limit);
        let query_vec = self.embedding.generate_embedding(query).await?;

        let filter_ids: Option<Vec<KnowledgeDocumentId>> = if collections.is_empty() {
            None
        } else {
            let ids = self
                .repos
                .knowledge_document
                .ids_by_collections(collections)
                .await?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            Some(ids)
        };

        let vector_leg = self
            .repos
            .knowledge_chunk
            .search_by_embedding(&query_vec, filter_ids.as_deref(), pool as i64)
            .await?;
        let lexical_leg = self
            .lexical_leg(query, filter_ids.as_deref(), pool as i64)
            .await?;

        let distances: HashMap<KnowledgeChunkId, f64> = vector_leg.iter().copied().collect();
        let vector_ids: Vec<KnowledgeChunkId> = vector_leg.into_iter().map(|(id, _)| id).collect();
        let fused = reciprocal_rank_fusion(vec![vector_ids, lexical_leg], |id| *id, pool);
        if fused.is_empty() {
            return Ok(Vec::new());
        }

        let hits = self.hydrate(fused, &distances).await?;
        self.score_and_trim(query, hits, limit).await
    }

    /// The lexical leg: chunks whose content carries any of the query's terms, most matches first.
    async fn lexical_leg(
        &self,
        query: &str,
        filter_ids: Option<&[KnowledgeDocumentId]>,
        limit: i64,
    ) -> Result<Vec<KnowledgeChunkId>> {
        let terms = lexical_terms(query, MAX_LEXICAL_TERMS);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        self.repos
            .knowledge_chunk
            .search_keywords(&terms, filter_ids, limit)
            .await
    }

    /// Joins the fused candidates to their chunks and documents, keeping the fused order together
    /// with each fusion score.
    async fn hydrate(
        &self,
        fused: Vec<(KnowledgeChunkId, f64)>,
        distances: &HashMap<KnowledgeChunkId, f64>,
    ) -> Result<Vec<(RelatedChunk, f64)>> {
        let chunk_ids: Vec<KnowledgeChunkId> = fused.iter().map(|(id, _)| *id).collect();
        let chunks = self.repos.knowledge_chunk.by_ids(&chunk_ids).await?;
        let doc_ids: Vec<KnowledgeDocumentId> = chunks.iter().map(|c| c.document_id).collect();
        let docs = self.repos.knowledge_document.by_ids(&doc_ids).await?;

        let doc_map: HashMap<KnowledgeDocumentId, KnowledgeDocument> =
            docs.into_iter().map(|d| (d.id, d)).collect();

        Ok(
            hydrate::merge_hydrated(fused, chunks.into_iter().map(|c| (c.id, c)).collect())
                .into_iter()
                .filter_map(|(chunk, fusion)| {
                    let doc = match doc_map.get(&chunk.document_id) {
                        Some(doc) => doc,
                        None => {
                            tracing::warn!(chunk = %chunk.id, "chunk has no document; skipping");
                            return None;
                        }
                    };
                    Some((
                        RelatedChunk {
                            chunk_id: chunk.id,
                            document_id: doc.id,
                            content: chunk.content.clone(),
                            document_title: doc.title.clone(),
                            collection: doc.collection.clone(),
                            distance: distances.get(&chunk.id).copied(),
                            score: None,
                        },
                        fusion,
                    ))
                })
                .collect(),
        )
    }

    /// Reorders the fused candidates by the scorer's scores, keeps the best chunk of each document
    /// and cuts the result to `limit`; without a scorer the fused order is the answer.
    async fn score_and_trim(
        &self,
        query: &str,
        hits: Vec<(RelatedChunk, f64)>,
        limit: i64,
    ) -> Result<Vec<RelatedChunk>> {
        let Some(scorer) = &self.scorer else {
            return Ok(best_per_document(hits)
                .into_iter()
                .take(limit as usize)
                .map(|(hit, _)| hit)
                .collect());
        };
        let ordered = rank::rerank(
            scorer,
            query,
            hits,
            |hit| Cow::Borrowed(hit.content.as_str()),
            ScorerErrorPolicy::Fail,
            "knowledge",
        )
        .await?;
        Ok(best_per_document(ordered)
            .into_iter()
            .take(limit as usize)
            .map(|(hit, score)| RelatedChunk {
                score: score.is_finite().then_some(score),
                ..hit
            })
            .collect())
    }

    /// Re-chunks and re-embeds every document whose `meta.chunker_version` is not the current
    /// one, returning how many were rebuilt. Distinct from `db rebuild embeddings`, which only
    /// recomputes vectors.
    pub async fn reindex(&self, collection: Option<&Collection>, cfg: &ChunkCfg) -> Result<usize> {
        let docs = self.repos.knowledge_document.list(collection).await?;

        let mut rebuilt = 0usize;
        for doc in docs {
            if parse_doc_meta(&doc).is_some_and(|m| m.chunker_version == CHUNKER_VERSION) {
                continue;
            }
            // Re-cut from the stored source text, which is the truth for derived chunks, and
            // force past the hash check so a stale version is rebuilt.
            self.upsert_document(
                &doc.source,
                &doc.title,
                &doc.collection,
                &doc.content,
                cfg,
                true,
            )
            .await?;
            rebuilt += 1;
        }
        Ok(rebuilt)
    }
}

fn best_per_document(hits: Vec<(RelatedChunk, f64)>) -> Vec<(RelatedChunk, f64)> {
    let mut documents = HashSet::new();
    hits.into_iter()
        .filter(|(hit, _)| documents.insert(hit.document_id))
        .collect()
}

/// Unparsable meta counts as absent, which forces a rebuild instead of silently keeping corrupt
/// data.
fn parse_doc_meta(doc: &KnowledgeDocument) -> Option<KnowledgeDocumentMeta> {
    let json = doc.meta.as_ref()?.clone();
    match serde_json::from_value(json) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!(doc_id = %doc.id, error = %e, "knowledge document meta is corrupted");
            None
        }
    }
}

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(hex, "{byte:02x}").expect("writing to String never fails");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(document: u128, chunk: u128) -> (RelatedChunk, f64) {
        (
            RelatedChunk {
                chunk_id: KnowledgeChunkId(uuid::Uuid::from_u128(chunk)),
                document_id: KnowledgeDocumentId(uuid::Uuid::from_u128(document)),
                content: format!("chunk {chunk}"),
                document_title: format!("document {document}"),
                collection: Collection::Uncategorised,
                distance: None,
                score: None,
            },
            1.0,
        )
    }

    /// Neighbouring chunks of one document must not crowd out the other documents behind them.
    #[test]
    fn one_limit_keeps_one_chunk_per_document() {
        let ranked = vec![hit(1, 11), hit(1, 12), hit(2, 21), hit(1, 13), hit(3, 31)];

        let kept: Vec<u128> = best_per_document(ranked)
            .into_iter()
            .map(|(hit, _)| hit.chunk_id.0.as_u128())
            .collect();
        assert_eq!(kept, [11, 21, 31]);
    }
}
