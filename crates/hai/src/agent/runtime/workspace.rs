//! The chat's own directory on the host.

use std::path::{Path, PathBuf};

use crate::{config::Paths, domain::vo::ChatId, error::Result};

/// The directory a chat's tools work in.
#[derive(Debug)]
pub struct Workspace {
    dir: PathBuf,
}

impl Workspace {
    /// A chat's workspace is named here and nowhere else.
    pub fn for_chat(chat_id: ChatId) -> Self {
        Self {
            dir: Paths::inferred().workspaces_dir().join(chat_id.to_string()),
        }
    }

    pub async fn create(chat_id: ChatId) -> Result<Self> {
        let workspace = Self::for_chat(chat_id);
        tokio::fs::create_dir_all(workspace.dir()).await?;
        Ok(workspace)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_gets_its_own_directory_under_the_workspaces_root() {
        let first = Workspace::for_chat(ChatId(4242));
        assert_eq!(first.dir(), Paths::inferred().workspaces_dir().join("4242"));
        assert_ne!(first.dir(), Workspace::for_chat(ChatId(4243)).dir());
    }
}
