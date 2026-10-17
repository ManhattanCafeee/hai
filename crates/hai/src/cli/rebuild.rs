use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use sqlx::PgPool;
use tokio::sync::Semaphore;

use crate::{
    agentcore::api::embedding::{EmbeddingApi, EmbeddingService},
    config::{AppConfig, ProviderRegistry, model_role},
    domain::{db, repo::vector},
    error::{AppError, Result},
};

const MAX_CONCURRENT: usize = 10;

pub async fn rebuild_embeddings(config: &AppConfig) -> Result<()> {
    let (pool, dimension) = db::init_db_with_dimension(config).await?;
    let registry = ProviderRegistry::discover(config).await;
    let svc: Arc<dyn EmbeddingService> = Arc::new(EmbeddingApi::new(model_role::embedding(
        config, &registry,
    )?)?);

    vector::reset_embedding_dimension(&pool, dimension).await?;

    let jobs = vector::collect_embedding_jobs(&pool).await?;
    let total = jobs.len();
    let pb = progress_bar("embeddings", total);
    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT));

    let failed = run_batch(svc, jobs, pool.clone(), sem, &pb).await;

    pb.finish_and_clear();
    let ok = total.saturating_sub(failed);
    println!("Written {ok}/{total} embeddings.");

    vector::rebuild_embedding_indexes(&pool, total).await?;
    println!("Done. Rebuilt embeddings ({dimension}d).");

    if failed > 0 {
        Err(AppError::message(format!(
            "{failed}/{total} embeddings failed"
        )))
    } else {
        Ok(())
    }
}

async fn run_batch(
    svc: Arc<dyn EmbeddingService>,
    jobs: Vec<vector::EmbeddingJob>,
    pool: PgPool,
    sem: Arc<Semaphore>,
    pb: &ProgressBar,
) -> usize {
    let mut stream: FuturesUnordered<_> = jobs
        .into_iter()
        .map(move |job| {
            let svc = Arc::clone(&svc);
            let sem = Arc::clone(&sem);
            let pool = pool.clone();
            async move {
                let _permit = sem
                    .acquire()
                    .await
                    .map_err(|_| AppError::message("semaphore closed"))?;
                let emb = svc.generate_embedding(&job.content).await.map_err(|e| {
                    AppError::message(format!("{}/{}: {e}", job.table.name(), job.id))
                })?;
                vector::upsert_embedding_vec(&pool, job.table, job.id, &emb).await?;
                Ok::<_, crate::error::AppError>(())
            }
        })
        .collect();

    let mut failed = 0usize;
    while let Some(result) = stream.next().await {
        if let Err(e) = result {
            tracing::warn!("{e}");
            failed += 1;
        }
        pb.inc(1);
    }
    failed
}

fn progress_bar(name: &str, total: usize) -> ProgressBar {
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::with_template("{prefix:>12} {bar:30.green} {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("━ "),
    );
    pb.set_prefix(name.to_owned());
    pb
}
