//! The platform's downloaded-file cache: bytes kept under the platform's own file key, so the same
//! handle is fetched once. It saves a download; it is not storage, so entries outlive their use only
//! until the next sweep.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use crate::error::Result;

/// How long an entry may sit unused.
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

const SWEEP_EVERY: u64 = 64;

#[derive(Debug)]
pub struct FileCache {
    cache_dir: PathBuf,
    writes: AtomicU64,
}

impl FileCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            writes: AtomicU64::new(0),
        }
    }

    /// An unreadable entry is a miss; `add` overwrites it.
    pub async fn find(&self, key: &str) -> Option<Vec<u8>> {
        tokio::fs::read(self.key_to_path(key)).await.ok()
    }

    pub async fn add(&self, key: &str, data: &[u8]) -> Result<()> {
        let path = self.key_to_path(key);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, data).await?;
        // The first write sweeps too, so a process that downloads rarely still cleans up.
        if self
            .writes
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(SWEEP_EVERY)
        {
            self.sweep(MAX_AGE).await;
        }
        Ok(())
    }

    /// Removes entries not written for `max_age`. Best effort: the cache's whole purpose is to save
    /// work, so a sweep that cannot run is not the caller's problem.
    async fn sweep(&self, max_age: Duration) {
        let Ok(mut entries) = tokio::fs::read_dir(&self.cache_dir).await else {
            return;
        };
        let cutoff = std::time::SystemTime::now()
            .checked_sub(max_age)
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let mut removed = 0usize;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let stale = entry
                .metadata()
                .await
                .is_ok_and(|meta| meta.modified().is_ok_and(|modified| modified < cutoff));
            if stale && tokio::fs::remove_file(entry.path()).await.is_ok() {
                removed += 1;
            }
        }
        if removed > 0 {
            tracing::debug!(removed, "swept the platform file cache");
        }
    }

    fn key_to_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join(sanitize_filename::sanitize(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    /// One key is one file whatever the platform's id spelling carries: a key that turned into a path
    /// under the cache directory would scatter entries and let one platform's id reach another's.
    #[tokio::test]
    async fn a_key_is_one_entry_and_comes_back() {
        let dir = TempDir::create("hai-cache-test");
        let cache = FileCache::new(dir.path().to_path_buf());
        let key = "telegram-AbC/../files:99";

        assert!(cache.find(key).await.is_none(), "nothing cached yet");

        cache
            .add(key, b"the bytes")
            .await
            .expect("a writable cache");
        assert_eq!(cache.find(key).await.as_deref(), Some(&b"the bytes"[..]));

        let entries = std::fs::read_dir(&cache.cache_dir)
            .expect("the cache directory")
            .count();
        assert_eq!(entries, 1, "one key is one file");
    }

    /// The sweep is what keeps the cache from growing forever; a zero age removes everything, which is
    /// what the test can drive without waiting a week.
    #[tokio::test]
    async fn a_sweep_removes_what_is_older_than_the_limit() {
        let dir = TempDir::create("hai-cache-test");
        let cache = FileCache::new(dir.path().to_path_buf());
        cache
            .add("telegram-1", b"bytes")
            .await
            .expect("a writable cache");

        cache.sweep(Duration::ZERO).await;

        assert!(
            cache.find("telegram-1").await.is_none(),
            "a swept entry is gone"
        );
    }
}
