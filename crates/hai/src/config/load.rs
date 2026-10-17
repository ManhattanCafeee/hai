use std::path::Path;

use crate::{
    config::{AppConfig, env::ENV_PREFIX, issue::ConfigIssue},
    error::{AppError, Result, bail},
};

impl AppConfig {
    /// Defaults, then the file, then `HAI__*` — one source per key. A missing file is the
    /// defaults; an unknown key in either source fails startup.
    pub fn load(path: &Path) -> Result<Self> {
        let mut builder = config::Config::builder();
        match std::fs::read_to_string(path) {
            Ok(raw) => {
                builder = builder.add_source(config::File::from_str(&raw, config::FileFormat::Toml))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                bail!(AppError::message(format!(
                    "failed to read config {}: {e}",
                    path.display()
                )));
            }
        }
        let cfg = builder
            .add_source(
                config::Environment::with_prefix(ENV_PREFIX)
                    .separator("__")
                    .convert_case(config::Case::Kebab),
            )
            .build()?
            .try_deserialize::<Self>()
            .map_err(|e| {
                AppError::message(format!("failed to load config {}: {e}", path.display()))
            })?;
        cfg.validate()?;
        Ok(cfg)
    }
}

impl AppConfig {
    /// Cross-field invariants the deserializer cannot express.
    fn validate(&self) -> Result<()> {
        if self.knowledge.chunk_max == 0 {
            bail!(AppError::from(ConfigIssue::new(
                "knowledge.chunk-max",
                "must be at least 1"
            )));
        }
        if self.knowledge.chunk_size == 0 {
            bail!(AppError::from(ConfigIssue::new(
                "knowledge.chunk-size",
                "must be at least 1"
            )));
        }
        if self.knowledge.chunk_overlap >= self.knowledge.chunk_size {
            bail!(AppError::from(ConfigIssue::new(
                "knowledge.chunk-overlap",
                "must be smaller than knowledge.chunk-size"
            )));
        }
        if self.agent.retrieval.candidates == 0 {
            bail!(AppError::from(ConfigIssue::new(
                "agent.retrieval.candidates",
                "must be at least 1"
            )));
        }
        if self.agent.retrieval.candidate_chars == 0 {
            bail!(AppError::from(ConfigIssue::new(
                "agent.retrieval.candidate-chars",
                "must be at least 1"
            )));
        }
        if self.agent.retrieval.timeout_secs == 0 {
            bail!(AppError::from(ConfigIssue::new(
                "agent.retrieval.timeout-secs",
                "must be at least 1"
            )));
        }
        for mount in &self.sandbox.mounts {
            if !mount.is_absolute() {
                bail!(AppError::from(ConfigIssue::new(
                    "sandbox.mounts",
                    format!("{} is not an absolute path", mount.display())
                )));
            }
            if !mount.exists() {
                bail!(AppError::from(ConfigIssue::new(
                    "sandbox.mounts",
                    format!("{} does not exist", mount.display())
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{Mutex, MutexGuard},
    };

    use super::AppConfig;

    /// Loading reads the process environment, so these tests run one at a time.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn temp_toml(name: &str, content: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn kebab_case_keys_still_load() {
        let _guard = env_lock();
        let path = temp_toml(
            "hai_cfg_valid.toml",
            "[agent.context]\ncontext-seed-cap = 7\n",
        );
        let cfg = AppConfig::load(&path).unwrap();
        assert_eq!(cfg.agent.context.context_seed_cap, 7);
    }

    #[test]
    fn unknown_key_fails_startup() {
        let _guard = env_lock();
        let path = temp_toml(
            "hai_cfg_stale.toml",
            "[agent.context]\nsystem-prompt = \"x\"\n",
        );
        assert!(AppConfig::load(&path).is_err());
    }

    #[test]
    fn zero_chunk_max_fails_startup() {
        let _guard = env_lock();
        let path = temp_toml("hai_cfg_chunk_max.toml", "[knowledge]\nchunk-max = 0\n");
        assert!(AppConfig::load(&path).is_err());
    }

    #[test]
    fn retrieval_budgets_and_chunk_overlap_must_be_usable() {
        let _guard = env_lock();
        for (name, content) in [
            (
                "hai_cfg_retrieval_candidates.toml",
                "[agent.retrieval]\ncandidates = 0\n",
            ),
            (
                "hai_cfg_retrieval_candidate_chars.toml",
                "[agent.retrieval]\ncandidate-chars = 0\n",
            ),
            (
                "hai_cfg_retrieval_timeout.toml",
                "[agent.retrieval]\ntimeout-secs = 0\n",
            ),
            (
                "hai_cfg_chunk_overlap.toml",
                "[knowledge]\nchunk-overlap = 512\n",
            ),
        ] {
            let path = temp_toml(name, content);
            assert!(AppConfig::load(&path).is_err(), "{name} must fail startup");
        }
    }

    #[test]
    fn removed_personality_description_fails_startup() {
        let _guard = env_lock();
        let path = temp_toml(
            "hai_cfg_desc.toml",
            "[agent.personality]\ndescription = \"x\"\n",
        );
        assert!(AppConfig::load(&path).is_err());
    }

    #[test]
    fn missing_file_is_all_defaults() {
        let _guard = env_lock();
        let path = std::env::temp_dir().join("hai_cfg_missing.toml");
        let cfg = AppConfig::load(&path).unwrap();
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn empty_file_is_all_defaults() {
        let _guard = env_lock();
        let path = temp_toml("hai_cfg_empty.toml", "");
        let cfg = AppConfig::load(&path).unwrap();
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn env_override_still_applies() {
        let _guard = env_lock();
        let path = temp_toml("hai_cfg_env.toml", "");
        unsafe { std::env::set_var("HAI__MODEL_ROLES__DEFAULT__MODEL", "test-model") };
        let cfg = AppConfig::load(&path).unwrap();
        unsafe { std::env::remove_var("HAI__MODEL_ROLES__DEFAULT__MODEL") };
        assert_eq!(cfg.model_roles.default.model, "test-model");
    }

    #[test]
    fn env_multi_word_key_maps_to_kebab_field() {
        let _guard = env_lock();
        let path = temp_toml("hai_cfg_env_kebab.toml", "");
        unsafe { std::env::set_var("HAI__MODEL_ROLES__DEFAULT__MAX_TOKENS", "1234") };
        let cfg = AppConfig::load(&path).unwrap();
        unsafe { std::env::remove_var("HAI__MODEL_ROLES__DEFAULT__MAX_TOKENS") };
        assert_eq!(cfg.model_roles.default.max_tokens, Some(1234));
    }

    /// The file's other fields of the same section survive an environment override of one key.
    #[test]
    fn env_overrides_one_field_without_touching_its_section() {
        let _guard = env_lock();
        let path = temp_toml(
            "hai_cfg_env_merge.toml",
            "[model-roles.default]\nreasoning = false\n",
        );
        unsafe { std::env::set_var("HAI__MODEL_ROLES__DEFAULT__TEMPERATURE", "0.7") };
        let cfg = AppConfig::load(&path).unwrap();
        unsafe { std::env::remove_var("HAI__MODEL_ROLES__DEFAULT__TEMPERATURE") };
        assert!(
            !cfg.model_roles.default.reasoning,
            "the file's reasoning value was replaced"
        );
        assert!((cfg.model_roles.default.temperature - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn unknown_env_key_fails_startup() {
        let _guard = env_lock();
        let path = temp_toml("hai_cfg_env_unknown.toml", "");
        unsafe { std::env::set_var("HAI__AGENT__MODLE", "1") };
        let result = AppConfig::load(&path);
        unsafe { std::env::remove_var("HAI__AGENT__MODLE") };
        assert!(result.is_err());
    }
}
