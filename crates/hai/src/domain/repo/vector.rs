//! Vector storage: the pgvector extension and one `embedding` column per table, the writes that
//! keep a row's vector in step with the row, and the rebuild that recomputes them all.

use std::fmt::Write;

use strum::{EnumIter, IntoEnumIterator, IntoStaticStr};
use uuid::Uuid;

use crate::{
    domain::{
        model::{KnowledgeChunk, Memory, Perception, Topic, embed_text},
        vo::ChatId,
    },
    error::{AppError, Result, bail},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum EmbeddingTable {
    Memory,
    Topic,
    Perception,
    KnowledgeChunk,
}

impl EmbeddingTable {
    pub fn name(self) -> &'static str {
        self.into()
    }

    /// The rows of this table a rebuild must embed, in the batch's order. The match is exhaustive,
    /// so a table added to `EmbeddingTable` cannot be left out of the rebuild.
    async fn jobs(self, pool: &sqlx::PgPool) -> Result<Vec<EmbeddingJob>> {
        let failed =
            |e: sqlx::Error| AppError::message(format!("Failed to query {}: {e}", self.name()));
        match self {
            EmbeddingTable::Memory => {
                let rows: Vec<Memory> = sqlx::query_as::<_, Memory>("SELECT * FROM memory")
                    .fetch_all(pool)
                    .await
                    .map_err(failed)?;
                Ok(rows
                    .into_iter()
                    .map(|row| EmbeddingJob {
                        table: self,
                        id: row.id.0,
                        content: row.content,
                    })
                    .collect())
            }
            EmbeddingTable::Topic => {
                let rows: Vec<Topic> =
                    sqlx::query_as::<_, Topic>("SELECT * FROM topic WHERE summary IS NOT NULL")
                        .fetch_all(pool)
                        .await
                        .map_err(failed)?;
                Ok(rows
                    .into_iter()
                    .filter_map(|row| {
                        let summary = row.summary?;
                        if summary.is_empty() {
                            return None;
                        }
                        Some(EmbeddingJob {
                            table: self,
                            id: row.id.0,
                            content: embed_text(row.title.as_deref().unwrap_or_default(), &summary),
                        })
                    })
                    .collect())
            }
            EmbeddingTable::Perception => {
                let rows: Vec<Perception> =
                    sqlx::query_as::<_, Perception>("SELECT * FROM perception")
                        .fetch_all(pool)
                        .await
                        .map_err(failed)?;
                Ok(rows
                    .into_iter()
                    .map(|row| EmbeddingJob {
                        table: self,
                        id: row.id.0,
                        content: row.content,
                    })
                    .collect())
            }
            EmbeddingTable::KnowledgeChunk => {
                let rows: Vec<KnowledgeChunk> =
                    sqlx::query_as::<_, KnowledgeChunk>("SELECT * FROM knowledge_chunk")
                        .fetch_all(pool)
                        .await
                        .map_err(failed)?;
                Ok(rows
                    .into_iter()
                    .map(|row| EmbeddingJob {
                        table: self,
                        id: row.id.0,
                        content: row.content,
                    })
                    .collect())
            }
        }
    }
}

/// One row a rebuild must embed: the table it lives in, its id, and the text to embed for it.
pub struct EmbeddingJob {
    pub table: EmbeddingTable,
    pub id: Uuid,
    pub content: String,
}

/// Every row a rebuild must embed, one table at a time in `EmbeddingTable::iter()` order — memory,
/// topic (summary present and non-empty), perception, knowledge chunk. The first half of
/// `hai db rebuild embeddings`; the table list is the enum itself, written down nowhere else.
pub async fn collect_embedding_jobs(pool: &sqlx::PgPool) -> Result<Vec<EmbeddingJob>> {
    let mut jobs = Vec::new();
    for table in EmbeddingTable::iter() {
        jobs.extend(table.jobs(pool).await?);
    }
    Ok(jobs)
}

/// Format a vector as the pgvector text literal (`[0.1,0.2]`) that `$n::vector` accepts, into one
/// buffer: every embedding written carries hundreds of numbers, so a `String` per number is real.
pub fn vec_to_pgstring(v: &[f32]) -> String {
    let mut out = String::with_capacity(v.len() * 12 + 2);
    out.push('[');
    for (index, value) in v.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write!(out, "{value}").expect("writing to String never fails");
    }
    out.push(']');
    out
}

async fn create_extension(pool: &sqlx::PgPool) -> Result<()> {
    sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
        .execute(pool)
        .await?;
    Ok(())
}

/// The declared width of `{table}.embedding` (`vector(N)`), or `None` when the column does not
/// exist.
async fn column_dimension(pool: &sqlx::PgPool, table: &str) -> Result<Option<i32>> {
    let ty: Option<String> = sqlx::query_scalar(
        "SELECT format_type(atttypid, atttypmod) FROM pg_attribute \
         WHERE attrelid = to_regclass($1) AND attname = 'embedding' \
           AND attnum > 0 AND NOT attisdropped",
    )
    .bind(table)
    .fetch_optional(pool)
    .await?;
    Ok(ty.as_deref().and_then(parse_vector_dim))
}

fn parse_vector_dim(ty: &str) -> Option<i32> {
    ty.strip_prefix("vector(")?.strip_suffix(')')?.parse().ok()
}

/// Bring the pgvector extension and every embedding column to `dimension`, adding missing columns.
/// A populated column of a different width cannot convert and the call errors, naming the rebuild
/// that rewrites it (`hai db rebuild embeddings`).
pub async fn ensure_embedding_schema(pool: &sqlx::PgPool, dimension: i32) -> Result<()> {
    create_extension(pool).await?;

    for table in EmbeddingTable::iter() {
        let table = table.name();
        sqlx::query(sqlx::AssertSqlSafe(
            format!("ALTER TABLE {table} ADD COLUMN IF NOT EXISTS embedding vector({dimension})")
                .as_str(),
        ))
        .execute(pool)
        .await?;
        let current = column_dimension(pool, table).await?.ok_or_else(|| {
            AppError::message(format!(
                "{table}.embedding has no declared width: run `hai db rebuild embeddings`"
            ))
        })?;
        if current == dimension {
            continue;
        }
        sqlx::query(sqlx::AssertSqlSafe(
            format!(
                "ALTER TABLE {table} ALTER COLUMN embedding TYPE vector({dimension}) USING embedding::vector"
            )
            .as_str(),
        ))
        .execute(pool)
        .await
        .map_err(|e| {
            AppError::message(format!(
                "{table}.embedding is vector({current}); changing it to vector({dimension}) fails with rows present ({e}): run `hai db rebuild embeddings`"
            ))
        })?;
    }
    Ok(())
}

/// Drop and recreate every embedding column at `dimension`, discarding the stored vectors — the
/// destructive first half of `hai db rebuild embeddings`, which recomputes them.
pub async fn reset_embedding_dimension(pool: &sqlx::PgPool, dimension: i32) -> Result<()> {
    create_extension(pool).await?;

    for table in EmbeddingTable::iter() {
        let table = table.name();
        sqlx::query(sqlx::AssertSqlSafe(
            format!("ALTER TABLE {table} DROP COLUMN IF EXISTS embedding").as_str(),
        ))
        .execute(pool)
        .await?;
        sqlx::query(sqlx::AssertSqlSafe(
            format!("ALTER TABLE {table} ADD COLUMN embedding vector({dimension})").as_str(),
        ))
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Fail when a column is missing or its width differs from `dimension`, naming the command that
/// fixes it. Called from `agent/runtime/context.rs:assemble`.
pub async fn assert_embedding_dimension(pool: &sqlx::PgPool, dimension: i32) -> Result<()> {
    for table in EmbeddingTable::iter() {
        let table = table.name();
        match column_dimension(pool, table).await? {
            Some(current) if current == dimension => {}
            Some(current) => bail!(AppError::message(format!(
                "{table}.embedding is vector({current}) but the configured embedding dimension is {dimension}: run `hai db rebuild embeddings`"
            ))),
            None => bail!(AppError::message(format!(
                "{table}.embedding is missing: run `hai db migrate`"
            ))),
        }
    }
    Ok(())
}

/// The tables a chat-scoped search can target: each carries `chat_id`.
#[derive(Debug, Clone, Copy)]
pub enum ChatTable {
    Memory,
    Topic,
}

impl ChatTable {
    fn table(self) -> EmbeddingTable {
        match self {
            ChatTable::Memory => EmbeddingTable::Memory,
            ChatTable::Topic => EmbeddingTable::Topic,
        }
    }
}

/// A predicate narrowing a search, as data the table's repo owns: `column` is a literal, and
/// `value` is bound, so a row filter never spells a wire value into SQL.
#[derive(Debug, Clone, Copy)]
pub struct Filter<'a> {
    pub column: &'static str,
    pub value: &'a str,
}

/// The `WHERE` body of one search together with the values it numbered. `$1` is the query vector
/// and `$2` the limit, so the filters take `$3` upward in field order.
struct SearchFilters<'a> {
    clause: String,
    chat_id: Option<ChatId>,
    document_ids: Option<&'a [Uuid]>,
    filter: Option<Filter<'a>>,
}

impl<'a> SearchFilters<'a> {
    fn chat(chat_id: ChatId, filter: Option<Filter<'a>>) -> Self {
        Self {
            clause: clause(&["chat_id = $3".to_owned()], filter),
            chat_id: Some(chat_id),
            document_ids: None,
            filter,
        }
    }

    fn documents(document_ids: &'a [Uuid]) -> Self {
        Self {
            clause: clause(&["document_id = ANY($3)".to_owned()], None),
            chat_id: None,
            document_ids: Some(document_ids),
            filter: None,
        }
    }

    fn unscoped() -> Self {
        Self {
            clause: clause(&[], None),
            chat_id: None,
            document_ids: None,
            filter: None,
        }
    }

    /// Binds in the order the clause numbered the placeholders.
    fn bind<'q>(self, mut query: SearchQuery<'q>) -> SearchQuery<'q>
    where
        'a: 'q,
    {
        if let Some(chat_id) = self.chat_id {
            query = query.bind(chat_id.0);
        }
        if let Some(ids) = self.document_ids {
            query = query.bind(ids);
        }
        if let Some(filter) = self.filter {
            query = query.bind(filter.value);
        }
        query
    }
}

/// `filter.column` is a literal the table's repo owns — never user input.
fn clause(conditions: &[String], filter: Option<Filter<'_>>) -> String {
    let mut parts: Vec<String> = conditions.to_vec();
    if let Some(filter) = filter {
        parts.push(format!("{} = $4", filter.column));
    }
    parts.push("embedding IS NOT NULL".to_owned());
    parts.join(" AND ")
}

type SearchQuery<'q> =
    sqlx::query::QueryAs<'q, sqlx::Postgres, (Uuid, f64), sqlx::postgres::PgArguments>;

/// L2 (`<->`) nearest-neighbor search, returning the top-k `(id, distance)`.
async fn search(
    pool: &sqlx::PgPool,
    table: EmbeddingTable,
    query: &[f32],
    filters: SearchFilters<'_>,
    limit: i64,
) -> Result<Vec<(Uuid, f64)>> {
    let table = table.name();
    let qv = vec_to_pgstring(query);
    let sql = format!(
        "SELECT id, embedding <-> $1::vector AS distance \
         FROM {table} \
         WHERE {} \
         ORDER BY embedding <-> $1::vector \
         LIMIT $2",
        filters.clause
    );
    let q = filters.bind(
        sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(qv)
            .bind(limit),
    );
    Ok(q.fetch_all(pool).await?)
}

/// Nearest neighbours inside one chat; only the tables that carry `chat_id` are accepted. `filter`
/// is the caller's row predicate, which a repo builds from its own table's vocabulary.
pub async fn search_chat_embedding_vec(
    pool: &sqlx::PgPool,
    table: ChatTable,
    query: &[f32],
    chat_id: ChatId,
    filter: Option<Filter<'_>>,
    limit: i64,
) -> Result<Vec<(Uuid, f64)>> {
    search(
        pool,
        table.table(),
        query,
        SearchFilters::chat(chat_id, filter),
        limit,
    )
    .await
}

/// Nearest neighbours inside the given documents. Chunks are the only embedding table that hangs
/// off a document, so the table is not a parameter.
pub async fn search_document_embedding_vec(
    pool: &sqlx::PgPool,
    query: &[f32],
    document_ids: &[Uuid],
    limit: i64,
) -> Result<Vec<(Uuid, f64)>> {
    search(
        pool,
        EmbeddingTable::KnowledgeChunk,
        query,
        SearchFilters::documents(document_ids),
        limit,
    )
    .await
}

/// Nearest neighbours with no row narrowing beyond the vector match.
pub async fn search_unscoped_embedding_vec(
    pool: &sqlx::PgPool,
    table: EmbeddingTable,
    query: &[f32],
    limit: i64,
) -> Result<Vec<(Uuid, f64)>> {
    search(pool, table, query, SearchFilters::unscoped(), limit).await
}

pub async fn upsert_embedding_vec<P: Into<Uuid>>(
    pool: &sqlx::PgPool,
    table: EmbeddingTable,
    id: P,
    emb: &[f32],
) -> Result<()> {
    let table = table.name();
    let sql = format!("UPDATE {table} SET embedding = $1::vector WHERE id = $2");
    sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(vec_to_pgstring(emb))
        .bind(id.into())
        .execute(pool)
        .await?;
    Ok(())
}

/// Recreates every embedding index over the rows a rebuild just wrote. The definition is recreated
/// rather than assumed because it has to match the `<->` L2 operator the retrieval queries use, and
/// Ivfflat needs rows to build on, so this runs after the writes, never before — the second half of
/// `hai db rebuild embeddings`.
pub async fn rebuild_embedding_indexes(pool: &sqlx::PgPool, total: usize) -> Result<()> {
    let lists = index_lists(total);
    for table in EmbeddingTable::iter() {
        let [drop, create] = embedding_index_sql(table, lists);
        sqlx::query(sqlx::AssertSqlSafe(drop.as_str()))
            .execute(pool)
            .await?;
        sqlx::query(sqlx::AssertSqlSafe(create.as_str()))
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// IVFFlat's list count for `total` rows: the square root, never below ten.
fn index_lists(total: usize) -> usize {
    (10usize).max((total as f64).sqrt() as usize)
}

/// The two statements that recreate one table's embedding index, in the order they run.
fn embedding_index_sql(table: EmbeddingTable, lists: usize) -> [String; 2] {
    let table = table.name();
    [
        format!("DROP INDEX IF EXISTS idx_{table}_embedding"),
        format!(
            "CREATE INDEX idx_{table}_embedding ON {table} \
             USING ivfflat (embedding vector_l2_ops) WITH (lists = {lists})"
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedding_table_is_named_and_ordered_for_the_rebuild() {
        let names: Vec<&str> = EmbeddingTable::iter().map(EmbeddingTable::name).collect();
        assert_eq!(names, ["memory", "topic", "perception", "knowledge_chunk"]);
    }

    #[test]
    fn the_index_is_recreated_with_the_operator_retrieval_uses() {
        let [drop, create] = embedding_index_sql(EmbeddingTable::KnowledgeChunk, 12);
        assert_eq!(drop, "DROP INDEX IF EXISTS idx_knowledge_chunk_embedding");
        assert_eq!(
            create,
            "CREATE INDEX idx_knowledge_chunk_embedding ON knowledge_chunk \
             USING ivfflat (embedding vector_l2_ops) WITH (lists = 12)"
        );
    }

    #[test]
    fn the_list_count_is_the_square_root_never_below_ten() {
        assert_eq!(index_lists(0), 10);
        assert_eq!(index_lists(99), 10);
        assert_eq!(index_lists(100), 10);
        assert_eq!(index_lists(400), 20);
    }

    #[test]
    fn each_scope_numbers_its_own_placeholder() {
        let chat = ChatId(7);
        let docs = [Uuid::from_u128(1), Uuid::from_u128(2)];
        let docs = docs.as_slice();
        let active = Filter {
            column: "status",
            value: "active",
        };

        let memory = SearchFilters::chat(chat, Some(active));
        assert_eq!(
            memory.clause,
            "chat_id = $3 AND status = $4 AND embedding IS NOT NULL"
        );
        assert_eq!(memory.chat_id, Some(chat));
        assert_eq!(memory.filter.map(|filter| filter.value), Some("active"));
        assert!(memory.document_ids.is_none());

        let topic = SearchFilters::chat(chat, None);
        assert_eq!(topic.clause, "chat_id = $3 AND embedding IS NOT NULL");

        let document_scope = SearchFilters::documents(docs);
        assert_eq!(
            document_scope.clause,
            "document_id = ANY($3) AND embedding IS NOT NULL"
        );
        assert!(document_scope.chat_id.is_none());
        assert_eq!(document_scope.document_ids, Some(docs));

        let unscoped = SearchFilters::unscoped();
        assert_eq!(unscoped.clause, "embedding IS NOT NULL");
        assert!(unscoped.chat_id.is_none());
        assert!(unscoped.document_ids.is_none());
    }
}
