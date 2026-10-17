//! The address a tool path names: one scheme table, so `read` and `bash` spell a location alike.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};
use uuid::Uuid;

use crate::{
    agent::{runtime::sandbox::Sandbox, web::WebUrl},
    agentcore::{
        skills::SkillName,
        tool::{ToolError, tool_err},
    },
    domain::vo::{AttachmentId, MemoryId, TopicId},
};

const FILE_SCHEME: &str = "file://";
const WORKSPACE_SCHEME: &str = "workspace://";
const ATTACHMENT_SCHEME: &str = "attachment://";
const SKILL_SCHEME: &str = "skill://";
const TOPIC_SCHEME: &str = "topic://";
const MEMORY_SCHEME: &str = "memory://";

/// The address a tool path names, parsed once at the boundary.
#[derive(Debug)]
pub(crate) enum Address {
    /// A path on the agent's filesystem: `file://<path>`, or a bare path.
    Local(PathBuf),
    /// A path inside the workspace: `workspace://<rel>`.
    Workspace(PathBuf),
    Attachment(AttachmentId),
    Skill(SkillName),
    Topic(TopicId),
    Memory(MemoryId),
    Url(WebUrl),
}

/// The directory a `workdir` may name: somewhere a command can start in.
#[derive(Debug)]
pub(crate) enum Directory {
    Local(PathBuf),
    Workspace(PathBuf),
}

/// A `path` argument in its wire form: still one string, but a type by the time the tool sees it.
pub(crate) fn deserialize_address<'de, D: Deserializer<'de>>(d: D) -> Result<Address, D::Error> {
    let raw = String::deserialize(d)?;
    parse(&raw).map_err(serde::de::Error::custom)
}

/// A `workdir` argument in its wire form, absent when the field is.
pub(crate) fn deserialize_workdir<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Directory>, D::Error> {
    let raw = Option::<String>::deserialize(d)?;
    raw.as_deref()
        .map(directory)
        .transpose()
        .map_err(serde::de::Error::custom)
}

/// A `source` argument in its wire form, absent when the field is.
pub(crate) fn deserialize_optional_address<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Address>, D::Error> {
    let raw = Option::<String>::deserialize(d)?;
    raw.as_deref()
        .map(parse)
        .transpose()
        .map_err(serde::de::Error::custom)
}

/// `path` → its address; a scheme no tool accepts is an error naming the ones they do.
pub(crate) fn parse(path: &str) -> Result<Address, ToolError> {
    let path = path.trim();
    if let Some(rest) = path.strip_prefix(FILE_SCHEME) {
        return Ok(Address::Local(PathBuf::from(rest)));
    }
    if let Some(rest) = path.strip_prefix(WORKSPACE_SCHEME) {
        return Ok(Address::Workspace(PathBuf::from(rest)));
    }
    if let Some(id) = path.strip_prefix(ATTACHMENT_SCHEME) {
        return Ok(Address::Attachment(AttachmentId(id_of(id, "attachment")?)));
    }
    if let Some(name) = path.strip_prefix(SKILL_SCHEME) {
        return SkillName::new(name)
            .map(Address::Skill)
            .ok_or_else(|| tool_err("skill:// needs a name"));
    }
    if let Some(id) = path.strip_prefix(TOPIC_SCHEME) {
        return Ok(Address::Topic(TopicId(id_of(id, "topic")?)));
    }
    if let Some(id) = path.strip_prefix(MEMORY_SCHEME) {
        return Ok(Address::Memory(MemoryId(id_of(id, "memory")?)));
    }
    if let Some((scheme, _)) = path.split_once("://") {
        let normalized = scheme.to_ascii_lowercase();
        if normalized != "http" && normalized != "https" {
            return Err(tool_err(format!(
                "Unsupported scheme: {scheme}; use file://, workspace://, http(s)://, attachment://, skill://, topic:// or memory://"
            )));
        }
        return WebUrl::parse(path)
            .map(Address::Url)
            .map_err(|e| tool_err(e.to_string()));
    }
    Ok(Address::Local(PathBuf::from(path)))
}

/// `path` → the directory a command may start in; every other address is not a directory.
fn directory(path: &str) -> Result<Directory, ToolError> {
    match parse(path)? {
        Address::Local(path) => Ok(Directory::Local(path)),
        Address::Workspace(rel) => Ok(Directory::Workspace(rel)),
        _ => Err(tool_err(format!("{path} is not a directory"))),
    }
}

fn id_of(raw: &str, kind: &str) -> Result<Uuid, ToolError> {
    Uuid::parse_str(raw.trim()).map_err(|e| tool_err(format!("Invalid {kind} id: {e}")))
}

/// The path a workspace-relative address names; the workspace's location lives in the sandbox, so a
/// caller hands its root in and the model's own spelling outlives any move of the directory.
pub(crate) fn resolve_workspace(root: &Path, rel: &Path) -> PathBuf {
    root.join(rel.strip_prefix("/").unwrap_or(rel))
}

/// The path on the host that a local address names: a relative path means the workspace, and an
/// absolute path under the environment's own root (a container's `/workspace`) is the same file as the
/// host's workspace, so the model's two spellings stay one file.
fn host_path(command_root: &Path, host_root: &Path, path: &Path) -> PathBuf {
    if let Ok(rel) = path.strip_prefix(command_root) {
        return host_root.join(rel);
    }
    if path.is_absolute() {
        return path.to_path_buf();
    }
    resolve_workspace(host_root, path)
}

impl Address {
    /// The host path this address names, when it names one: `workspace://` and a local path share one
    /// mapping, so every tool that opens a file reaches the same one.
    pub(crate) fn local_path(&self, sandbox: &Sandbox) -> Option<PathBuf> {
        match self {
            Self::Local(path) => Some(host_path(
                sandbox.command_root(),
                sandbox.host_workspace(),
                path,
            )),
            Self::Workspace(rel) => Some(resolve_workspace(sandbox.host_workspace(), rel)),
            _ => None,
        }
    }
}

impl Directory {
    /// The path a command starts in, as the environment spells it: a relative one resolves against the
    /// environment's own root, an absolute one stands.
    pub(crate) fn command_path(&self, root: &Path) -> PathBuf {
        match self {
            Self::Local(path) if path.is_absolute() => path.clone(),
            Self::Local(path) => resolve_workspace(root, path),
            Self::Workspace(rel) => resolve_workspace(root, rel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::runtime::workspace::Workspace, config::schema::SandboxConfig, domain::vo::ChatId,
    };

    #[test]
    fn a_bare_path_is_the_local_scheme() {
        assert!(matches!(parse("a.md"), Ok(Address::Local(path)) if path == Path::new("a.md")));
        assert!(
            matches!(parse("  ./notes.md  "), Ok(Address::Local(path)) if path == Path::new("./notes.md"))
        );
        // A colon that is not a scheme keeps the path local.
        assert!(
            matches!(parse("/tmp/a:b"), Ok(Address::Local(path)) if path == Path::new("/tmp/a:b"))
        );
        assert!(
            matches!(parse("file:///tmp/a.md"), Ok(Address::Local(path)) if path == Path::new("/tmp/a.md"))
        );
        assert!(
            matches!(parse("file://rel"), Ok(Address::Local(path)) if path == Path::new("rel"))
        );
    }

    #[test]
    fn the_workspace_scheme_names_the_workspace() {
        assert!(
            matches!(parse("workspace://n/x.md"), Ok(Address::Workspace(path)) if path == Path::new("n/x.md"))
        );
        assert!(
            matches!(parse("workspace://"), Ok(Address::Workspace(path)) if path == Path::new(""))
        );
    }

    #[test]
    fn the_other_schemes_keep_their_meaning() {
        assert!(matches!(
            parse("attachment://3f0c1a9e-1b2c-4d5e-8f90-112233445566"),
            Ok(Address::Attachment(_))
        ));
        assert!(
            matches!(parse("skill://demo"), Ok(Address::Skill(name)) if name.as_str() == "demo")
        );
        assert!(matches!(
            parse("topic://3f0c1a9e-1b2c-4d5e-8f90-112233445566"),
            Ok(Address::Topic(_))
        ));
        assert!(matches!(
            parse("memory://3f0c1a9e-1b2c-4d5e-8f90-112233445566"),
            Ok(Address::Memory(_))
        ));
        assert!(matches!(
            parse("HTTP://Example.COM/a?b=1"),
            Ok(Address::Url(url)) if url.as_str() == "http://example.com/a?b=1"
        ));
    }

    #[test]
    fn an_unknown_scheme_lists_the_known_ones() {
        let err = parse("ftp://host/x").expect_err("rejected");
        assert!(err.to_string().contains("workspace://"));
        assert!(parse("http://").is_err());
        // A dangling id is the id's own error, not the scheme table's.
        assert!(parse("topic://nope").is_err());
    }

    #[test]
    fn a_workdir_is_a_directory_or_nothing() {
        assert!(matches!(
            directory("workspace://n"),
            Ok(Directory::Workspace(path)) if path == Path::new("n")
        ));
        assert!(matches!(directory("notes"), Ok(Directory::Local(_))));
        assert_eq!(
            directory("skill://demo").expect_err("rejected").to_string(),
            "skill://demo is not a directory"
        );
    }

    #[test]
    fn the_workspace_root_resolves_relative_addresses() {
        assert_eq!(
            resolve_workspace(Path::new("/workspace"), Path::new("n/x.md")),
            PathBuf::from("/workspace/n/x.md")
        );
        assert_eq!(
            resolve_workspace(Path::new("/workspace"), Path::new("/n/x.md")),
            PathBuf::from("/workspace/n/x.md")
        );
        assert_eq!(
            resolve_workspace(Path::new("/w"), Path::new("")),
            PathBuf::from("/w")
        );
    }

    #[test]
    fn the_local_address_is_re_rooted_onto_the_host() {
        let command_root = Path::new("/workspace");
        let host_root = Path::new("/data/ws/7");

        // A relative path means the workspace; the environment's own root is its other name.
        assert_eq!(
            host_path(command_root, host_root, Path::new("a.txt")),
            PathBuf::from("/data/ws/7/a.txt")
        );
        assert_eq!(
            host_path(command_root, host_root, Path::new("/workspace/a.txt")),
            PathBuf::from("/data/ws/7/a.txt")
        );
        // Anything else absolute keeps its meaning and is left for the sandbox to refuse.
        assert_eq!(
            host_path(command_root, host_root, Path::new("/etc/passwd")),
            PathBuf::from("/etc/passwd")
        );
        // Without a container the two roots are the same directory.
        assert_eq!(
            host_path(Path::new("/w"), Path::new("/w"), Path::new("/w/a.txt")),
            PathBuf::from("/w/a.txt")
        );
    }

    #[test]
    fn a_local_address_shares_the_workspace_mapping() {
        let host = Sandbox::host(Workspace::for_chat(ChatId(9)), 30);
        let container = Sandbox::container(
            &SandboxConfig::default(),
            Vec::new(),
            Workspace::for_chat(ChatId(9)),
        );

        // A relative path means the workspace, whatever the environment calls it.
        assert_eq!(
            parse("a.txt").expect("address").local_path(&host),
            Some(host.host_workspace().join("a.txt"))
        );
        assert_eq!(
            parse("/workspace/a.txt")
                .expect("address")
                .local_path(&container),
            Some(container.host_workspace().join("a.txt"))
        );
        // Anything else absolute keeps its meaning and is left for the sandbox to refuse.
        assert_eq!(
            parse("/etc/passwd").expect("address").local_path(&host),
            Some(PathBuf::from("/etc/passwd"))
        );
        assert_eq!(
            parse("skill://demo").expect("address").local_path(&host),
            None
        );
    }

    #[test]
    fn a_workdir_is_spelled_as_the_environment_sees_it() {
        let root = Path::new("/workspace");

        assert_eq!(
            directory("notes").expect("workdir").command_path(root),
            PathBuf::from("/workspace/notes")
        );
        assert_eq!(
            directory("workspace://notes")
                .expect("workdir")
                .command_path(root),
            PathBuf::from("/workspace/notes")
        );
        assert_eq!(
            directory("/etc").expect("workdir").command_path(root),
            PathBuf::from("/etc")
        );
    }
}
