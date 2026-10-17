use std::path::Path;

use crate::{
    config::meta::PROJECT_NAME_UPPER,
    error::{AppError, Result},
};

pub const ENV_PREFIX: &str = PROJECT_NAME_UPPER;

/// An empty or blank environment value counts as unconfigured, not as a credential.
pub fn read_env_key(env: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    env(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn local_mode() -> bool {
    std::env::var(format!("{}_LOCAL_MODE", ENV_PREFIX)).is_ok()
}

pub fn load_dotenv(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    dotenvy::from_path(path)
        .map_err(|e| AppError::message(format!("failed to load env file {}: {e}", path.display())))
}
