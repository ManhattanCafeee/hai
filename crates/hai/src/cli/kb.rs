use std::path::{Path, PathBuf};

use crate::{
    config::AppConfig,
    domain::{
        service::{Services, chunking::ChunkCfg},
        vo::{Collection, KnowledgeDocumentId},
    },
    error::{AppError, Result, bail},
};

const IMPORT_EXTENSIONS: [&str; 2] = ["md", "txt"];

/// The CLI spells a collection as a plain label; the empty string is the uncategorised bucket.
fn collection_of(label: &str) -> Collection {
    label
        .parse::<Collection>()
        .expect("Collection accepts every label")
}

pub async fn build_services(config: &AppConfig) -> Result<Services> {
    Ok(crate::agent::runtime::assemble(config).await?.services)
}

pub fn chunk_cfg(config: &AppConfig) -> ChunkCfg {
    ChunkCfg {
        size: config.knowledge.chunk_size,
        overlap: config.knowledge.chunk_overlap,
        max: config.knowledge.chunk_max,
    }
}

pub async fn execute(action: super::KbAction, config: &AppConfig) -> Result<()> {
    let services = build_services(config).await?;
    let cfg = chunk_cfg(config);
    match action {
        super::KbAction::Import {
            paths,
            collection,
            title,
            recursive,
        } => {
            import(
                &services,
                &paths,
                collection.as_deref(),
                title.as_deref(),
                recursive,
                config,
            )
            .await
        }
        super::KbAction::List { collection } => list(&services, collection.as_deref()).await,
        super::KbAction::Search {
            query,
            collection,
            limit,
        } => search(&services, &query, collection.as_deref(), limit).await,
        super::KbAction::Delete { id } => {
            services.knowledge.delete(KnowledgeDocumentId(id)).await?;
            println!("Deleted document {id}");
            Ok(())
        }
        super::KbAction::Reindex { collection } => {
            let collection = collection.as_deref().map(collection_of);
            let n = services
                .knowledge
                .reindex(collection.as_ref(), &cfg)
                .await?;
            println!("Reindexed {n} document(s).");
            Ok(())
        }
    }
}

async fn import(
    services: &Services,
    paths: &[PathBuf],
    collection: Option<&str>,
    title_override: Option<&str>,
    recursive: bool,
    config: &AppConfig,
) -> Result<()> {
    let cfg = chunk_cfg(config);
    let files = collect_files(paths, recursive)?;
    if files.is_empty() {
        bail!(AppError::message("no importable files (.md/.txt) found"));
    }

    let bar = if files.len() > 1 {
        Some(indicatif::ProgressBar::new(files.len() as u64))
    } else {
        None
    };

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut failures: Vec<(String, String)> = Vec::new();

    for path in &files {
        let result = import_one(services, path, collection, title_override, &cfg).await;
        match result {
            Ok(true) => imported += 1,
            Ok(false) => skipped += 1,
            Err(e) => failures.push((path.display().to_string(), e.to_string())),
        }
        if let Some(b) = &bar {
            b.inc(1);
        }
    }
    if let Some(b) = bar {
        b.finish_and_clear();
    }

    println!(
        "Imported {imported}, skipped {skipped} (unchanged), failed {}.",
        failures.len()
    );
    if !config.knowledge.enabled {
        println!(
            "  note: [knowledge] enabled = false — the agent will not mount search_knowledge_base"
        );
    }
    for (path, err) in &failures {
        println!("  failed: {path}: {err}");
    }
    if !failures.is_empty() {
        return Err(AppError::message(format!(
            "{} file(s) failed to import",
            failures.len()
        )));
    }
    Ok(())
}

/// Returns `true` when the document was written, `false` when its content was
/// unchanged and the file was skipped.
async fn import_one(
    services: &Services,
    path: &Path,
    collection: Option<&str>,
    title_override: Option<&str>,
    cfg: &ChunkCfg,
) -> Result<bool> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| AppError::message(format!("cannot read {}: {e}", path.display())))?;
    let (front_title, body) = extract_frontmatter(&content);
    let title = title_override
        .map(String::from)
        .or(front_title)
        .unwrap_or_else(|| stem_name(path));
    let source = path.display().to_string();
    let collection = collection_of(collection.unwrap_or(""));
    let outcome = services
        .knowledge
        .upsert_document(&source, &title, &collection, &body, cfg, false)
        .await?;
    if outcome.imported {
        println!("  imported: {title} ({} chunks)", outcome.chunk_count);
    }
    Ok(outcome.imported)
}

fn collect_files(paths: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = Vec::new();
    for p in paths {
        let meta = std::fs::metadata(p)
            .map_err(|e| AppError::message(format!("cannot access {}: {e}", p.display())))?;
        if meta.is_file() {
            if !is_importable(p) {
                bail!(AppError::message(format!(
                    "'{}' is not an importable file (.md/.txt)",
                    p.display()
                )));
            }
            files.push(p.clone());
        } else if meta.is_dir() {
            if !recursive {
                bail!(AppError::message(format!(
                    "'{}' is a directory; use --recursive to import recursively",
                    p.display()
                )));
            }
            walk_dir(p, &mut files)?;
        }
    }
    files.sort();
    Ok(files)
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)
        .map_err(|e| AppError::message(format!("cannot read dir {}: {e}", dir.display())))?
    {
        let entry = entry.map_err(|e| AppError::message(e.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, out)?;
        } else if is_importable(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_importable(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMPORT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn stem_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

/// Returns the frontmatter title with the body stripped of the block, or the
/// input unchanged when there is no frontmatter.
fn extract_frontmatter(content: &str) -> (Option<String>, String) {
    let Some(rest) = content.strip_prefix("---") else {
        return (None, content.to_string());
    };
    let Some(end) = rest.find("\n---") else {
        return (None, content.to_string());
    };
    let head = &rest[..end];
    if !head.lines().any(|l| l.split_once(':').is_some()) {
        return (None, content.to_string());
    }
    let title = head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        if k.trim() == "title" {
            Some(v.trim().trim_matches('"').trim_matches('\'').to_string())
        } else {
            None
        }
    });
    let body_start = end + 4;
    let body = rest.get(body_start..).unwrap_or("");
    (title, body.trim_start_matches('\n').to_string())
}

async fn list(services: &Services, collection: Option<&str>) -> Result<()> {
    let collection = collection.map(collection_of);
    let docs = services.knowledge.list(collection.as_ref()).await?;
    if docs.is_empty() {
        println!("(empty)");
        return Ok(());
    }
    for doc in docs {
        let col = match &doc.collection {
            Collection::Uncategorised => "-",
            named => named.as_str(),
        };
        println!("{:<38}  {:<24}  {}", doc.id, col, doc.title);
    }
    Ok(())
}

async fn search(
    services: &Services,
    query: &str,
    collection: Option<&str>,
    limit: i64,
) -> Result<()> {
    let collections: Vec<Collection> = collection
        .map(|c| vec![collection_of(c)])
        .unwrap_or_default();
    let hits = services
        .knowledge
        .search(query, limit, &collections)
        .await?;
    if hits.is_empty() {
        println!("(no matches)");
        return Ok(());
    }
    for hit in hits {
        let col = match &hit.collection {
            Collection::Uncategorised => "-",
            named => named.as_str(),
        };
        let snippet: String = hit.content.chars().take(120).collect();
        let score = match (hit.score, hit.distance) {
            (Some(score), _) => format!("p={score:.4}"),
            (None, Some(distance)) => format!("d={distance:.4}"),
            (None, None) => "-".to_owned(),
        };
        println!(
            "[{col}] {} (id={}, {score})\n    {}",
            hit.document_title, hit.document_id.0, snippet
        );
    }
    Ok(())
}
