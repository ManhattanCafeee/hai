use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agentcore::Language;

/// Per-chat preferences; an absent field falls back to the configured default.
///
/// The blob is written and read by this program alone, so unknown fields are tolerated: a newer
/// build may add a preference an older build still has to read past.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ChatPreferences {
    pub language: Option<Language>,
}

impl ChatPreferences {
    pub fn with_language(mut self, language: Language) -> Self {
        self.language = Some(language);
        self
    }

    pub fn language_or(&self, default: Language) -> Language {
        self.language.unwrap_or(default)
    }

    pub fn to_value(&self) -> serde_json::Result<Value> {
        serde_json::to_value(self)
    }
}
