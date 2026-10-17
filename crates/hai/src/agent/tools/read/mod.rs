//! The `read` tool: every source it can open, the pagination it applies to a long one, and the
//! surface it advertises so the model can choose among them.

mod local;
mod media;
mod paging;
mod record;
mod skill;
mod url;

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use strum::IntoEnumIterator;

use self::paging::{Page, page_data};
use super::addressing::{self, Address};
use crate::{
    agent::{
        multimodal::{
            MultimodalService,
            analysis::{InlineLimit, MediaAnalyzer},
        },
        runtime::{AgentRuntime, sandbox::Sandbox, turn::ToolContext},
        web::{WebService, WebUrl},
    },
    agentcore::{
        skills::{SkillManager, SkillName},
        tool::{AgentTool, ToolError, schema_of, tool_data},
    },
    domain::vo::{AttachmentId, AttachmentParser, ChatId, MemoryId, TopicId},
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadArgs {
    /// file://<path> (a bare path means the same), workspace://<path>, http(s)://<url>, attachment://<uuid>, skill://<name>, topic://<id>, memory://<id>
    #[serde(deserialize_with = "addressing::deserialize_address")]
    #[schemars(with = "String")]
    pub path: Address,
    /// Starting position, 1-based, counted in the unit the result reports (`lines`, `chars` or `notes`)
    pub offset: Option<u64>,
    /// Maximum amount to read, counted in the unit the result reports (`lines`, `chars` or `notes`)
    pub limit: Option<u64>,
    /// Attachment parsers (image, video, audio): analyze against this instruction instead of the default full transcription
    pub focus: Option<String>,
    /// `ocr` reads an image as raw text instead of a description; any other value must match the file
    pub parser: Option<AttachmentParser>,
}

pub struct Read {
    description: String,
    chat_id: ChatId,
    web: WebService,
    analyzer: MediaAnalyzer,
    sandbox: Arc<Sandbox>,
    skill_manager: SkillManager,
}

enum ReadOutput {
    Text { content: String },
    Paged(Page),
    Skill { name: String, instructions: String },
}

#[async_trait]
impl AgentTool for Read {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Option<Value> {
        Some(schema_of::<ReadArgs>())
    }

    async fn execute(&self, args: Value) -> Result<Value, ToolError> {
        let args: ReadArgs = serde_json::from_value(args)?;
        let data = match self.read(&args).await? {
            ReadOutput::Text { content } => json!({ "content": content }),
            ReadOutput::Paged(page) => page_data(page),
            ReadOutput::Skill { name, instructions } => {
                json!({ "skill": name, "instructions": instructions })
            }
        };
        tool_data(data)
    }
}

enum Target<'a> {
    Attachment(AttachmentId),
    Skill(&'a SkillName),
    Topic(TopicId),
    Memory(MemoryId),
    Url(&'a WebUrl),
    Local(PathBuf),
}

/// The address resolved against the sandbox: where a read actually comes from.
fn target<'a>(address: &'a Address, sandbox: &Sandbox) -> Target<'a> {
    match address {
        Address::Local(_) | Address::Workspace(_) => Target::Local(
            address
                .local_path(sandbox)
                .expect("the arm matched an address that names a local path"),
        ),
        Address::Attachment(id) => Target::Attachment(*id),
        Address::Skill(name) => Target::Skill(name),
        Address::Topic(id) => Target::Topic(*id),
        Address::Memory(id) => Target::Memory(*id),
        Address::Url(url) => Target::Url(url),
    }
}

impl Read {
    async fn read(&self, args: &ReadArgs) -> Result<ReadOutput, ToolError> {
        let focus = args.focus.as_deref();
        match target(&args.path, &self.sandbox) {
            Target::Attachment(id) => self.read_attachment(id, args.parser, focus).await,
            Target::Skill(name) => skill::skill_body(&self.skill_manager, name),
            Target::Topic(id) => {
                record::topic(
                    self.analyzer.services(),
                    self.chat_id,
                    id,
                    args.offset,
                    args.limit,
                )
                .await
            }
            Target::Memory(id) => record::memory(self.analyzer.services(), self.chat_id, id).await,
            Target::Url(url) => self.read_url(url, args).await,
            Target::Local(path) => self.read_local(&path, args).await,
        }
    }
}

pub fn tools(runtime: &AgentRuntime, ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    let parsers = enabled_parsers(&ctx.multimodal);
    let inline_limit = InlineLimit::new(runtime.cfg().agent.multimodal.inline_max_bytes);
    vec![Arc::new(Read {
        description: describe(&parsers, !ctx.skill_manager.is_empty()),
        chat_id: ctx.chat.chat_id(),
        web: runtime.web.clone(),
        analyzer: MediaAnalyzer::new(
            ctx.multimodal.clone(),
            ctx.services.clone(),
            ctx.chat.clone(),
            inline_limit,
        ),
        sandbox: ctx.sandbox.clone(),
        skill_manager: ctx.skill_manager.clone(),
    })]
}

/// Lists only the sources this process can actually serve.
fn describe(parsers: &[AttachmentParser], skills: bool) -> String {
    let kinds: Vec<&str> = AttachmentParser::iter()
        .filter(|parser| parsers.contains(parser))
        .map(|parser| parser.name())
        .collect();
    let media = if kinds.is_empty() {
        String::new()
    } else {
        format!("; a media file is analyzed instead ({})", kinds.join("/"))
    };
    let mut lines = vec![
        "Reads content. `path` accepts:".to_owned(),
        format!(
            "- file://<path> (a bare path means the same; a directory is listed): text read line by line{media}"
        ),
        "- workspace://<path> (a relative path also resolves here): your workspace".to_owned(),
        format!("- http/https URL: the page converted to body text{media}"),
    ];
    if !parsers.is_empty() {
        lines.push(format!(
            "- attachment://<uuid>: an attachment in the chat (the id of `<attachment id=\"…\">`); parseable: {}",
            kinds.join("/")
        ));
    }
    if parsers.contains(&AttachmentParser::Image) {
        lines.push(
            "- `parser`: `ocr` reads an image as raw text instead of a description".to_owned(),
        );
    }
    if skills {
        lines.push("- skill://<name>: loads that skill's detailed instructions".to_owned());
    }
    lines.push(
        "- topic://<id>: topic archive (summary + message range + raw material; offset/limit count notes, everything by default)"
            .to_owned(),
    );
    lines.push("- memory://<id>: memory entry (content + status + source messages)".to_owned());
    lines.join("\n")
}

/// The attachment parsers the model is told about: only what the resolved service can actually read.
fn enabled_parsers(multimodal: &MultimodalService) -> Vec<AttachmentParser> {
    let mut parsers = Vec::new();
    if multimodal.transcription_enabled() {
        parsers.push(AttachmentParser::Audio);
    }
    if multimodal.vision_enabled() {
        parsers.push(AttachmentParser::Video);
        parsers.push(AttachmentParser::Image);
    }
    parsers
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{agent::runtime::workspace::Workspace, domain::vo::ChatId};

    /// The local path or the kind a `path` argument resolves to, as text so the borrow stays inside.
    fn resolved(path: &str, sandbox: &Sandbox) -> String {
        let address = addressing::parse(path).expect("address");
        match target(&address, sandbox) {
            Target::Local(path) => path.display().to_string(),
            Target::Attachment(_) => "attachment".to_owned(),
            Target::Skill(name) => format!("skill:{}", name.as_str()),
            Target::Topic(_) => "topic".to_owned(),
            Target::Memory(_) => "memory".to_owned(),
            Target::Url(url) => format!("url:{}", url.as_str()),
        }
    }

    #[test]
    fn a_target_resolves_local_addresses_against_the_workspace() {
        let workspace = Workspace::for_chat(ChatId(4242));
        let root = workspace.dir().display().to_string();
        let sandbox = Sandbox::host(workspace, 30);
        assert_eq!(resolved("a.md", &sandbox), format!("{root}/a.md"));
        assert_eq!(
            resolved("workspace://n/x.md", &sandbox),
            format!("{root}/n/x.md")
        );
        assert_eq!(resolved("file:///tmp/a.md", &sandbox), "/tmp/a.md");
        assert_eq!(
            resolved(
                "attachment://3f0c1a9e-1b2c-4d5e-8f90-112233445566",
                &sandbox
            ),
            "attachment"
        );
        assert_eq!(resolved("skill://demo", &sandbox), "skill:demo");
        assert_eq!(
            resolved("topic://3f0c1a9e-1b2c-4d5e-8f90-112233445566", &sandbox),
            "topic"
        );
        assert_eq!(
            resolved("memory://3f0c1a9e-1b2c-4d5e-8f90-112233445566", &sandbox),
            "memory"
        );
        assert_eq!(
            resolved("HTTP://Example.COM/a?b=1", &sandbox),
            "url:http://example.com/a?b=1"
        );
    }

    #[test]
    fn the_path_argument_arrives_typed() {
        let args: ReadArgs =
            serde_json::from_value(json!({ "path": "workspace://n/x.md" })).expect("args");
        assert!(matches!(args.path, Address::Workspace(path) if path == Path::new("n/x.md")));

        // The wire form is still one string, so a bad scheme or id fails where every argument does.
        let scheme = serde_json::from_value::<ReadArgs>(json!({ "path": "ftp://host/x" }))
            .expect_err("unknown scheme");
        assert!(scheme.to_string().contains("Unsupported scheme"));
        let id = serde_json::from_value::<ReadArgs>(json!({ "path": "topic://nope" }))
            .expect_err("bad id");
        assert!(id.to_string().contains("Invalid topic id"));
    }

    #[test]
    fn describe_lists_available_sources() {
        assert!(!describe(&[], false).contains("attachment://"));
        assert!(!describe(&[], false).contains("skill://"));
        assert!(describe(&[], true).contains("skill://<name>"));
        assert!(describe(&[], false).contains("file://<path>"));
        assert!(describe(&[], false).contains("workspace://"));
        assert!(
            describe(&[AttachmentParser::Image, AttachmentParser::Audio], false)
                .contains("image/audio")
        );
    }
}
