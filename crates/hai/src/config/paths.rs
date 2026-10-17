use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::config::{env, meta::PROJECT_NAME};

#[derive(Clone, Debug)]
pub struct Paths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    config_file: PathBuf,
    file_cache_dir: PathBuf,
    skill_dirs: Vec<PathBuf>,
}

impl Paths {
    pub fn inferred() -> &'static Self {
        static PATHS: OnceLock<Paths> = OnceLock::new();
        PATHS.get_or_init(Self::resolve)
    }

    fn resolve() -> Self {
        let local = PathBuf::from(format!(".{PROJECT_NAME}"));
        let use_local = local.exists() || env::local_mode();
        let config_dir = if use_local {
            local.clone()
        } else {
            dirs::config_dir()
                .map(|p| p.join(PROJECT_NAME))
                .unwrap_or_else(|| local.clone())
        };
        let data_dir = if use_local {
            local
        } else {
            dirs::data_dir()
                .map(|p| p.join(PROJECT_NAME))
                .unwrap_or(local)
        };

        let config_file = config_dir.join("config.toml");
        let file_cache_dir = data_dir.join("files");

        let mut skill_dirs = vec![config_dir.join("skills")];
        let local_dir = PathBuf::from(format!(".{PROJECT_NAME}"));
        let local_skills = local_dir.join("skills");
        if local_skills != config_dir.join("skills") {
            skill_dirs.push(local_skills);
        }
        skill_dirs.push(PathBuf::from(".agents/skills"));
        skill_dirs.retain(|d| d.exists());

        Self {
            config_dir,
            data_dir,
            config_file,
            file_cache_dir,
            skill_dirs,
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    pub fn env_file(&self) -> PathBuf {
        self.config_dir.join(".env")
    }

    pub fn file_cache_dir(&self) -> &Path {
        &self.file_cache_dir
    }

    /// The per-chat workspace roots: the one writable place a sandboxed agent has.
    pub fn workspaces_dir(&self) -> PathBuf {
        self.data_dir.join("workspaces")
    }

    pub fn skill_dirs(&self) -> &[PathBuf] {
        &self.skill_dirs
    }
}
