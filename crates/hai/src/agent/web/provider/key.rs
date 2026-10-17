use super::Spec;
use crate::config::{
    env::read_env_key,
    schema::{WebConfig, WebProvider},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeySource {
    /// Environment variable named by the table's `env_key` (e.g. `PARALLEL_API_KEY`)
    Env,
    /// `[web.providers.<id>] api-key` in the config file
    Config,
    /// Nothing configured
    None,
}

pub(super) struct Key {
    pub(super) value: Option<String>,
    pub(super) source: KeySource,
}
pub(super) fn resolve_key(
    cfg: &WebConfig,
    id: WebProvider,
    spec: &Spec,
    env: &impl Fn(&str) -> Option<String>,
) -> Key {
    if let Some(value) = read_env_key(env, spec.env_key) {
        return Key {
            value: Some(value),
            source: KeySource::Env,
        };
    }
    match cfg.providers.api_key(id).map(str::to_owned) {
        Some(value) => Key {
            value: Some(value),
            source: KeySource::Config,
        },
        None => Key {
            value: None,
            source: KeySource::None,
        },
    }
}
