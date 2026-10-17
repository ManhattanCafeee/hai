//! Locating a config problem: the TOML key path it lives at, and what is wrong there.

use std::fmt;

use crate::{agentcore::ModelKind, error::AppError};

/// A config problem at a TOML key path, so the message says where to edit.
pub struct ConfigIssue {
    path: String,
    detail: String,
}

impl ConfigIssue {
    pub fn new(path: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            detail: detail.into(),
        }
    }

    /// A problem inside the `[model-roles.<block>]` block a model kind resolves from.
    pub fn model_role(kind: ModelKind, detail: impl Into<String>) -> Self {
        Self::new(format!("model-roles.{}", model_role_block(kind)), detail)
    }

    /// A problem inside `[providers.<name>]`.
    pub fn provider(name: &str, detail: impl Into<String>) -> Self {
        Self::new(format!("providers.{name}"), detail)
    }

    /// A problem inside `[web.providers.<name>]`.
    pub fn web_provider(name: &str, detail: impl Into<String>) -> Self {
        Self::new(format!("web.providers.{name}"), detail)
    }
}

impl fmt::Display for ConfigIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.path, self.detail)
    }
}

impl From<ConfigIssue> for AppError {
    fn from(issue: ConfigIssue) -> Self {
        Self::Message(issue.to_string())
    }
}

/// The `[model-roles.<block>]` block a kind resolves from; the chat kind runs on `default`.
fn model_role_block(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Chat => "default",
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path names the kind's own block, and chat's block is the one keyed `default`.
    #[test]
    fn the_path_names_the_block() {
        assert_eq!(
            ConfigIssue::model_role(ModelKind::Embedding, "model is required").to_string(),
            "[model-roles.embedding] model is required"
        );
        assert_eq!(
            ConfigIssue::model_role(ModelKind::Chat, "no provider is named").to_string(),
            "[model-roles.default] no provider is named"
        );
        assert_eq!(
            ConfigIssue::provider("openai", "needs an api-key").to_string(),
            "[providers.openai] needs an api-key"
        );
        assert_eq!(
            ConfigIssue::web_provider("tavily", "requires a credential").to_string(),
            "[web.providers.tavily] requires a credential"
        );
    }
}
