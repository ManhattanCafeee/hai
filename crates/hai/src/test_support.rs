//! Test helpers the whole crate's modules share.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::Serialize;

use crate::{
    agentcore::api::rerank::RerankService,
    error::{AppError, Result},
};

/// The canned answer a [`FakeRerank`] serves: `Scores` returns them as-is, `Zeros` answers one
/// zero per candidate, `Down` is an outage.
#[derive(Debug)]
enum Reply {
    Scores(Vec<f64>),
    Zeros,
    Down,
}

/// A `RerankService` a test hands a canned answer to, counting the calls it served.
#[derive(Debug)]
pub(crate) struct FakeRerank {
    reply: Reply,
    calls: Mutex<usize>,
}

impl FakeRerank {
    pub(crate) fn scores(scores: Vec<f64>) -> Self {
        Self {
            reply: Reply::Scores(scores),
            calls: Mutex::new(0),
        }
    }

    pub(crate) fn zeros() -> Self {
        Self {
            reply: Reply::Zeros,
            calls: Mutex::new(0),
        }
    }

    pub(crate) fn down() -> Self {
        Self {
            reply: Reply::Down,
            calls: Mutex::new(0),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        *self.calls.lock().expect("not poisoned")
    }
}

#[async_trait::async_trait]
impl RerankService for FakeRerank {
    async fn rerank(&self, _query: &str, candidates: &[&str]) -> Result<Vec<f64>> {
        *self.calls.lock().expect("not poisoned") += 1;
        match &self.reply {
            Reply::Scores(scores) => Ok(scores.clone()),
            Reply::Zeros => Ok(vec![0.0; candidates.len()]),
            Reply::Down => Err(AppError::message("rerank is down")),
        }
    }
}

/// Asserts every variant's serde wire name matches the name its `strum` label carries: the two
/// spellings of one enum must not drift apart. Covers both a flat string wire form and an
/// internally tagged payload's `event` tag.
pub(crate) fn assert_names_agree<T: std::fmt::Debug + Serialize>(
    variants: impl IntoIterator<Item = T>,
    name: impl Fn(&T) -> &'static str,
) {
    for variant in variants {
        let wire = serde_json::to_value(&variant).expect("serializable");
        let tag = match &wire {
            serde_json::Value::String(tag) => tag.as_str(),
            serde_json::Value::Object(fields) => fields
                .get("event")
                .and_then(serde_json::Value::as_str)
                .expect("internally tagged enum carries its tag"),
            other => panic!("unexpected wire form {other}"),
        };
        assert_eq!(tag, name(&variant), "{variant:?}");
    }
}

/// A directory a test owns: dropping this removes the tree, so an assertion that panics still cleans
/// up.
#[derive(Debug)]
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    /// A unique directory under the system temp dir.
    pub(crate) fn create(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Self(dir)
    }

    /// A directory that already exists and that this guard takes over.
    pub(crate) fn adopt(dir: PathBuf) -> Self {
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
