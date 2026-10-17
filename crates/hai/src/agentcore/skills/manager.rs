use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use tracing::{debug, warn};

use crate::error::Result;

/// Make a relative path absolute against the process cwd: these roots become sandbox mounts and
/// the container runtime needs absolute paths.
fn absolutize(p: &Path) -> PathBuf {
    match std::env::current_dir() {
        Ok(cwd) if !p.is_absolute() => cwd.join(p),
        _ => p.to_path_buf(),
    }
}

/// The name a skill is addressed by, as it comes in over the wire.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkillName(String);

impl SkillName {
    /// A name is never empty or blank.
    pub fn new(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub disable_model_invocation: bool,
    pub body: String,
    pub base_dir: PathBuf,
}

impl Skill {
    pub fn body(&self) -> String {
        let base = self.base_dir.to_string_lossy();
        self.body.replace("{baseDir}", &base)
    }

    pub fn discovery_entry(&self) -> String {
        format!("\"{}\": {}", self.name, self.description)
    }
}

#[derive(Debug, Default, Clone)]
pub struct SkillManager {
    /// The loaded skills, fixed once load returns.
    skills: Arc<[Skill]>,
    /// Skill roots that exist on disk, collected at load and handed to the shell sandbox as
    /// mounts.
    roots: Vec<PathBuf>,
}

impl SkillManager {
    pub async fn load(dirs: &[PathBuf], disabled: &[String]) -> Result<Self> {
        let mut buf = Vec::new();
        let mut roots = Vec::new();
        for dir in dirs {
            let dir = absolutize(dir);
            if dir.exists() {
                Self::load_dir(&mut buf, &dir, disabled).await;
                roots.push(dir.clone());
            } else {
                debug!("Skills directory not found, skipping: {}", dir.display());
            }
        }
        Ok(Self {
            skills: buf.into(),
            roots,
        })
    }

    async fn load_dir(buf: &mut Vec<Skill>, dir: &Path, disabled: &[String]) {
        let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
            warn!("Failed to read skills directory: {}", dir.display());
            return;
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let Some(skill_file) = Self::find_skill_file(&path) else {
                continue;
            };

            let content = match tokio::fs::read_to_string(&skill_file).await {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to read {}: {e}", skill_file.display());
                    continue;
                }
            };

            let parsed = match agent_skills::Skill::parse(&content) {
                Ok(s) => s,
                Err(e) => {
                    warn!("Failed to parse {}: {e}", skill_file.display());
                    continue;
                }
            };

            let name = parsed.name().as_str().to_string();

            if disabled.contains(&name) {
                debug!("Skill '{}' is disabled, skipping", name);
                continue;
            }

            if buf.iter().any(|s| s.name == name) {
                debug!("Skill '{}' already loaded, skipping duplicate", name);
                continue;
            }

            let disable_model_invocation = parsed
                .frontmatter()
                .metadata()
                .and_then(|m| m.get("disable-model-invocation"))
                .and_then(|v| v.parse::<bool>().ok())
                .unwrap_or(false);

            buf.push(Skill {
                name,
                description: parsed.description().as_str().to_string(),
                disable_model_invocation,
                body: parsed.body_trimmed().to_string(),
                base_dir: path,
            });
        }
    }

    fn find_skill_file(dir: &Path) -> Option<PathBuf> {
        let candidates = [dir.join("SKILL.md"), dir.join("skill.md")];
        candidates.into_iter().find(|p| p.exists())
    }

    pub fn discoverable_skills(&self) -> impl Iterator<Item = &Skill> {
        self.skills.iter().filter(|s| !s.disable_model_invocation)
    }

    pub fn find(&self, name: &SkillName) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name.as_str())
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn discovery_prompt(&self) -> Option<String> {
        let entries: Vec<String> = self
            .discoverable_skills()
            .map(|s| format!("  - {}", s.discovery_entry()))
            .collect();

        if entries.is_empty() {
            return None;
        }

        Some(format!(
            "## Skills\n\
            The specialized capabilities (skills) below expose their full instructions through `read skill://<name>`. When a user request matches the description of a skill, read its instructions first and then follow them exactly:\n\
            {}\n\
            A {{baseDir}} inside those instructions is the directory of that skill.",
            entries.join("\n")
        ))
    }
}
