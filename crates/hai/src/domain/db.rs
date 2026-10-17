use std::str::FromStr;

use sqlx::PgPool;
use tracing::info;
use url::Url;

use crate::{
    config::{
        AppConfig,
        schema::{DatabaseConfig, resolve_embedding_dimension},
    },
    domain::repo::vector,
    error::{AppError, Result},
};

pub async fn init_db(config: &DatabaseConfig) -> Result<PgPool> {
    PgPool::connect_with(
        sqlx::postgres::PgConnectOptions::from_str(&config.url)
            .map_err(|e| crate::error::AppError::message(e.to_string()))?,
    )
    .await
    .map_err(Into::into)
}

/// Connects and resolves the configured embedding dimension: the pool the embedding commands
/// write through, and the width their columns must have.
pub async fn init_db_with_dimension(config: &AppConfig) -> Result<(PgPool, i32)> {
    let pool = init_db(&config.database).await?;
    let dimension = resolve_embedding_dimension(&config.model_roles.embedding)?;
    Ok((pool, dimension))
}

pub async fn apply_schema(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(include_str!("../../migrations/schema.sql"))
        .execute(pool)
        .await?;
    normalize_legacy_columns(pool).await?;
    Ok(())
}

/// The whole of `hai db migrate`: apply the schema, then bring every embedding column to the
/// configured dimension. Nothing runs automatically, so a deploy must run this.
pub async fn migrate(config: &AppConfig) -> Result<()> {
    let pool = init_db(&config.database).await?;
    apply_schema(&pool).await?;
    let dim = resolve_embedding_dimension(&config.model_roles.embedding)?;
    vector::ensure_embedding_schema(&pool, dim).await?;
    Ok(())
}

/// Converges a database on the single spelling the code uses: NULL `source_message_ids` and
/// `topic_note.message_ids` become the empty array the model reads, the columns every write path
/// binds become NOT NULL, and the DDL defaults that disagree with the write paths (`memory.status`,
/// the `now()` defaults on `account`/`topic` times) are dropped.
async fn normalize_legacy_columns(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(
        "UPDATE memory SET source_message_ids = '{}' WHERE source_message_ids IS NULL;\n\
         ALTER TABLE memory ALTER COLUMN source_message_ids SET DEFAULT '{}';\n\
         ALTER TABLE memory ALTER COLUMN source_message_ids SET NOT NULL;\n\
         ALTER TABLE memory ALTER COLUMN chat_id SET NOT NULL;\n\
         ALTER TABLE memory ALTER COLUMN status DROP DEFAULT;\n\
         UPDATE topic_note SET message_ids = '{}' WHERE message_ids IS NULL;\n\
         ALTER TABLE topic_note ALTER COLUMN message_ids SET DEFAULT '{}';\n\
         ALTER TABLE topic_note ALTER COLUMN message_ids SET NOT NULL;\n\
         ALTER TABLE account ALTER COLUMN last_active_at DROP DEFAULT;\n\
         ALTER TABLE topic ALTER COLUMN started_at DROP DEFAULT;\n\
         ALTER TABLE topic ALTER COLUMN last_active_at DROP DEFAULT;",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn create_database(url: &str) -> Result<()> {
    let (admin_url, db_name) = split_admin_url(url)?;
    let pool = PgPool::connect(&admin_url).await?;
    let sql = create_database_sql(&db_name)?;
    sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .execute(&pool)
        .await?;
    info!(%db_name, "Database created");
    Ok(())
}

/// The one statement `CREATE DATABASE` runs. A database name cannot be a bind parameter, so it is
/// validated and quoted here; `AssertSqlSafe` is sound because the name cannot carry a quote or a
/// path separator.
fn create_database_sql(db_name: &str) -> Result<String> {
    if db_name.is_empty() || db_name.contains(['"', '/', '\\']) {
        return Err(AppError::message(format!(
            "refusing to create a database named {db_name:?}"
        )));
    }
    Ok(format!("CREATE DATABASE \"{db_name}\""))
}

/// The admin URL — the same server and credentials, the `postgres` database — and the name the
/// target URL carries. A URL without a database name is refused here, not by the server.
fn split_admin_url(url: &str) -> Result<(String, String)> {
    let parsed =
        Url::parse(url).map_err(|e| AppError::message(format!("invalid database url: {e}")))?;
    let db_name = parsed.path().trim_start_matches('/').to_owned();
    if db_name.is_empty() {
        return Err(AppError::message(
            "the database url carries no database name",
        ));
    }
    let mut admin = parsed;
    admin.set_path("/postgres");
    Ok((admin.to_string(), db_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_admin_url_keeps_the_server_and_swaps_the_database() {
        assert_eq!(
            split_admin_url("postgresql://user:pw@localhost:5432/hai?sslmode=require")
                .expect("a url with a database name"),
            (
                "postgresql://user:pw@localhost:5432/postgres?sslmode=require".to_owned(),
                "hai".to_owned()
            )
        );
        assert_eq!(
            split_admin_url("postgres://localhost/hai").expect("a url with a database name"),
            ("postgres://localhost/postgres".to_owned(), "hai".to_owned())
        );
    }

    #[test]
    fn a_url_without_a_database_name_is_refused() {
        for url in [
            "postgresql://localhost",
            "postgresql://localhost/",
            "not a url",
        ] {
            assert!(
                split_admin_url(url).is_err(),
                "{url} carries no database name to create"
            );
        }
    }

    #[test]
    fn a_name_that_cannot_be_quoted_is_refused() {
        assert_eq!(
            create_database_sql("hai").expect("a plain name"),
            "CREATE DATABASE \"hai\""
        );
        for name in ["", "hai\"; DROP DATABASE hai; --", "a/b"] {
            assert!(
                create_database_sql(name).is_err(),
                "{name:?} cannot be quoted into an identifier"
            );
        }
    }
}
