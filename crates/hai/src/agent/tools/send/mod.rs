//! The `send_message` tool: the surface this channel and this build can serve, and the one call the
//! agent makes to put something in the chat.

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use self::generate::GenerateImage;
use super::{
    addressing::{Address, deserialize_optional_address},
    util::required,
};
use crate::{
    agent::{
        channel::{
            BuildCapability, ChannelCapabilities, ChannelHandle, OutboundForm, OutboundKind,
            OutboundPayload, ReplyTo,
        },
        multimodal::MultimodalService,
        runtime::{sandbox::Sandbox, turn::ToolContext},
    },
    agentcore::tool::{AgentTool, ToolError, schema_of, tool_err, tool_ok},
    domain::vo::{MessageId, TopicId},
};

mod generate;
mod media;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMessageArgs {
    pub kind: OutboundKind,
    pub content: Option<String>,
    pub prompt: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_address")]
    #[schemars(with = "Option<String>")]
    pub source: Option<Address>,
    #[schemars(with = "Option<Uuid>")]
    pub topic_id: Option<TopicId>,
    /// Internal id of the message this one points at, as shown in the context as `<msg id="N">`
    #[schemars(with = "Option<i64>")]
    pub reference: Option<MessageId>,
    /// A part of that message you are answering, copied verbatim from it
    pub quote: Option<String>,
}

pub struct SendMessage {
    pub(super) chat: ChannelHandle,
    pub(super) sandbox: Arc<Sandbox>,
    pub(super) multimodal: MultimodalService,
    /// What the platform declares, and the forms this build can serve from it: fixed for the life of
    /// the tool, so the surface is computed once instead of on every schema read.
    capabilities: ChannelCapabilities,
    forms: ServedForms,
}

impl SendMessage {
    pub(super) fn new(ctx: &ToolContext) -> Self {
        Self::of(
            ctx.chat.clone(),
            ctx.sandbox.clone(),
            ctx.multimodal.clone(),
        )
    }

    /// The declared surface is derived here and nowhere else: the platform declares the forms, this
    /// build declares which of them it can serve.
    fn of(chat: ChannelHandle, sandbox: Arc<Sandbox>, multimodal: MultimodalService) -> Self {
        let capabilities = chat.capabilities();
        let forms = ServedForms(
            capabilities
                .forms
                .iter()
                .copied()
                .filter(|form| match form.requires {
                    BuildCapability::None => true,
                    BuildCapability::Speech => multimodal.speech_enabled(),
                })
                .collect(),
        );
        Self {
            chat,
            sandbox,
            multimodal,
            capabilities,
            forms,
        }
    }
}

#[async_trait]
impl AgentTool for SendMessage {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send something to the chat. `kind` selects what goes out."
    }

    fn schema(&self) -> Option<Value> {
        Some(self.advertised_schema())
    }

    fn speaks(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value, ToolError> {
        let args: SendMessageArgs = serde_json::from_value(args)?;
        let (reply, payload) = self.outbound(args).await?;
        self.chat.send(reply, payload).await?;
        tool_ok()
    }
}

/// The forms this platform and this build can serve: the surface the send tool advertises and checks.
#[derive(Clone)]
struct ServedForms(Vec<OutboundForm>);

impl ServedForms {
    /// The `kind` guide the model reads, from the same declaration the runtime checks against.
    fn guide(&self) -> String {
        let lines: Vec<String> = self
            .0
            .iter()
            .map(|form| {
                let name: &'static str = form.kind.into();
                let fields: Vec<&str> = form.fields.iter().map(|field| field.name).collect();
                format!("`{name}` takes {}", fields.join(", "))
            })
            .collect();
        if lines.is_empty() {
            return "What goes out.".to_owned();
        }
        format!("What goes out. {}", lines.join("; "))
    }

    /// Whether any served form takes that argument.
    fn declares(&self, name: &str) -> bool {
        self.0
            .iter()
            .any(|form| form.fields.iter().any(|field| field.name == name))
    }

    fn get(&self, kind: OutboundKind) -> Option<OutboundForm> {
        self.0.iter().copied().find(|form| form.kind == kind)
    }
}

impl SendMessage {
    /// The schema the model sees: the forms this platform and this build can serve, and only the
    /// arguments those forms take.
    fn advertised_schema(&self) -> Value {
        let capabilities = self.capabilities;
        let forms = &self.forms;
        let mut schema = schema_of::<SendMessageArgs>();
        if let Some(kind) = schema.pointer_mut("/properties/kind") {
            let names: Vec<&'static str> = forms.0.iter().map(|form| form.kind.into()).collect();
            kind["enum"] = json!(names);
            kind["description"] = json!(forms.guide());
        }
        let mut described: Vec<&str> = Vec::new();
        for form in &forms.0 {
            for field in form.fields {
                if !described.contains(&field.name) {
                    described.push(field.name);
                    if let Some(property) =
                        schema.pointer_mut(&format!("/properties/{}", field.name))
                    {
                        property["description"] = json!(field.description);
                    }
                }
            }
        }
        for (field, kept) in [
            ("content", forms.declares("content")),
            ("prompt", forms.declares("prompt")),
            ("source", forms.declares("source")),
            ("reference", capabilities.reference),
            ("quote", capabilities.quote),
        ] {
            if !kept {
                drop_property(&mut schema, field);
            }
        }
        schema
    }

    /// The forms this build can serve: the platform declares them, this build declares what it can
    /// produce.
    fn served_forms(&self) -> ServedForms {
        self.forms.clone()
    }

    /// What leaves and what it points at: the chat is the handle's, so it is not built here.
    async fn outbound(
        &self,
        args: SendMessageArgs,
    ) -> Result<(ReplyTo, OutboundPayload), ToolError> {
        let kind: &'static str = args.kind.into();
        let forms = self.served_forms();
        let Some(form) = forms.get(args.kind) else {
            return Err(tool_err(format!("`{kind}` is not available here")));
        };
        self.reject_stray_fields(&args, &form)?;

        let reply = ReplyTo {
            topic_id: args.topic_id,
            reference: args.reference,
            quote: args.quote,
        };

        let payload = match args.kind {
            OutboundKind::Text => OutboundPayload::Text {
                content: required(args.content, "content", kind)?,
            },
            OutboundKind::Voice => {
                let prompt = required(args.prompt, "prompt", kind)?;
                let bytes = self.multimodal.speech(&prompt).await?;
                OutboundPayload::Voice {
                    bytes,
                    content: args.content,
                    prompt,
                }
            }
            OutboundKind::File => {
                let source = required(args.source, "source", kind)?;
                self.media_payload(source, args.content).await?
            }
        };

        Ok((reply, payload))
    }

    /// Refuses a field the form or the platform does not take, rather than ignoring it.
    fn reject_stray_fields(
        &self,
        args: &SendMessageArgs,
        form: &OutboundForm,
    ) -> Result<(), ToolError> {
        let kind: &'static str = args.kind.into();
        let capabilities = self.capabilities;
        if args.reference.is_some() && !capabilities.reference {
            return Err(tool_err("this platform takes no `reference`"));
        }
        if args.quote.is_some() && !capabilities.quote {
            return Err(tool_err("this platform takes no `quote`"));
        }
        for (field, given) in [
            ("content", args.content.is_some()),
            ("prompt", args.prompt.is_some()),
            ("source", args.source.is_some()),
        ] {
            if given && !form.fields.iter().any(|declared| declared.name == field) {
                return Err(tool_err(format!("`{kind}` takes no `{field}`")));
            }
        }
        Ok(())
    }
}

fn drop_property(schema: &mut Value, name: &str) {
    if let Some(properties) = schema
        .pointer_mut("/properties")
        .and_then(Value::as_object_mut)
    {
        properties.remove(name);
    }
}

pub fn tools(ctx: &ToolContext) -> Vec<Arc<dyn AgentTool>> {
    let mut all: Vec<Arc<dyn AgentTool>> = vec![Arc::new(SendMessage::new(ctx))];
    if ctx.multimodal.image_enabled() {
        all.push(Arc::new(GenerateImage {
            sandbox: ctx.sandbox.clone(),
            multimodal: ctx.multimodal.clone(),
        }));
    }
    all
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::{
        agent::{
            channel::{
                Channel, ChannelCapabilities, ChannelHandle, ChannelProfile, OutboundField,
                OutboundMessage, OutboundSource, TypingGuard,
            },
            runtime::{sandbox::Sandbox, workspace::Workspace},
        },
        config::{AppConfig, ProviderRegistry},
        domain::vo::{AccountId, ChannelKey, ChatId},
    };

    const TEXT_FIELDS: [OutboundField; 1] = [OutboundField {
        name: "content",
        description: "What to say",
    }];
    const VOICE_FIELDS: [OutboundField; 2] = [
        OutboundField {
            name: "prompt",
            description: "How it should be spoken",
        },
        OutboundField {
            name: "content",
            description: "The line that goes with it",
        },
    ];
    const FILE_FIELDS: [OutboundField; 2] = [
        OutboundField {
            name: "source",
            description: "An existing thing to send",
        },
        OutboundField {
            name: "content",
            description: "The line that goes with it",
        },
    ];
    const ALL_FORMS: [OutboundForm; 3] = [
        OutboundForm {
            kind: OutboundKind::Text,
            fields: &TEXT_FIELDS,
            requires: BuildCapability::None,
        },
        OutboundForm {
            kind: OutboundKind::Voice,
            fields: &VOICE_FIELDS,
            requires: BuildCapability::Speech,
        },
        OutboundForm {
            kind: OutboundKind::File,
            fields: &FILE_FIELDS,
            requires: BuildCapability::None,
        },
    ];
    const TEXT_AND_FILE: [OutboundForm; 2] = [
        OutboundForm {
            kind: OutboundKind::Text,
            fields: &TEXT_FIELDS,
            requires: BuildCapability::None,
        },
        OutboundForm {
            kind: OutboundKind::File,
            fields: &FILE_FIELDS,
            requires: BuildCapability::None,
        },
    ];

    /// A PNG's magic bytes, so a file's content decides the medium.
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    /// A channel that records what it was asked to send and declares the capabilities the test wants.
    struct RecordingChannel {
        capabilities: ChannelCapabilities,
        sent: Mutex<Option<OutboundMessage>>,
    }

    #[async_trait]
    impl Channel for RecordingChannel {
        fn key(&self) -> &ChannelKey {
            static KEY: std::sync::LazyLock<ChannelKey> =
                std::sync::LazyLock::new(|| ChannelKey::new("telegram", "test"));
            &KEY
        }

        fn profile(&self) -> ChannelProfile {
            ChannelProfile {
                account_id: AccountId(1),
                username: "test".to_owned(),
                name: "Test".to_owned(),
            }
        }

        fn capabilities(&self) -> ChannelCapabilities {
            self.capabilities
        }

        async fn send(&self, msg: OutboundMessage) -> crate::error::Result<()> {
            *self.sent.lock().expect("test channel lock") = Some(msg);
            Ok(())
        }

        async fn start_typing(&self, _chat_id: ChatId) -> Option<Box<dyn TypingGuard>> {
            None
        }

        async fn download_file(&self, _file_id: &str) -> crate::error::Result<Vec<u8>> {
            Ok(Vec::new())
        }

        async fn file_url(&self, _file_id: &str) -> crate::error::Result<String> {
            Ok(String::new())
        }
    }

    impl std::fmt::Debug for RecordingChannel {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("RecordingChannel").finish()
        }
    }

    fn capabilities(
        forms: &'static [OutboundForm],
        reference: bool,
        quote: bool,
    ) -> ChannelCapabilities {
        ChannelCapabilities {
            forms,
            reference,
            quote,
            max_media_bytes: 50 * 1024 * 1024,
        }
    }

    /// The config both services start from: a chat model with its provider registered, plus the
    /// embedding role the roles insist on.
    fn service_config(provider: &str, model: &str) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.model_roles.default.provider = provider.into();
        cfg.model_roles.default.model = model.into();
        cfg.providers.insert(provider.into(), Default::default());
        cfg.model_roles.embedding = crate::config::schema::EmbeddingConfig {
            provider: Some("openai".into()),
            model: Some("text-embedding-3-small".into()),
            dimension: None,
            base_url: None,
        };
        cfg.providers.insert("openai".into(), Default::default());
        cfg
    }

    /// A multimodal service over one config: the tests differ only in which roles it declares.
    fn service_of(cfg: &AppConfig) -> MultimodalService {
        let registry =
            ProviderRegistry::from_sources(cfg, &|_| Some("test-key".to_owned()), &|_| true);
        let roles = crate::config::model_role::resolve(cfg, &registry).expect("model roles");
        MultimodalService::from_config(cfg, &roles).expect("multimodal service")
    }

    /// A service whose provider cannot speak or generate: the local media paths stay reachable.
    fn service() -> MultimodalService {
        service_of(&service_config("anthropic", "claude-sonnet-4-5"))
    }

    /// The sandbox a tool under test acts in: the chat's own workspace, host-mode.
    fn host_sandbox() -> Arc<Sandbox> {
        Arc::new(Sandbox::host(Workspace::for_chat(ChatId(9)), 30))
    }

    fn tool(channel: Arc<dyn Channel>) -> SendMessage {
        SendMessage::of(
            ChannelHandle::new(channel, ChatId(42)),
            host_sandbox(),
            service(),
        )
    }

    fn recording_in(sandbox: Arc<Sandbox>) -> (SendMessage, Arc<RecordingChannel>) {
        let channel = Arc::new(RecordingChannel {
            capabilities: capabilities(&ALL_FORMS, true, true),
            sent: Mutex::new(None),
        });
        let tool = SendMessage::of(
            ChannelHandle::new(channel.clone(), ChatId(42)),
            sandbox,
            service(),
        );
        (tool, channel)
    }

    fn recording() -> (SendMessage, Arc<RecordingChannel>) {
        recording_in(host_sandbox())
    }

    fn sent_payload(channel: &RecordingChannel) -> OutboundPayload {
        let guard = channel.sent.lock().expect("test channel lock");
        let msg = guard.as_ref().expect("a send");
        msg.payload.clone()
    }

    /// A service that can speak: the voice form then needs `prompt` even without image generation.
    fn speaking_service() -> MultimodalService {
        let mut cfg = service_config("openai", "gpt-4o");
        cfg.model_roles.speech = Some(crate::config::schema::SpeechConfig {
            provider: None,
            model: Some("gpt-4o-mini-tts".into()),
            voice: "alloy".into(),
            speed: 1.0,
            base_url: None,
        });
        service_of(&cfg)
    }

    fn tool_with(multimodal: MultimodalService, capabilities: ChannelCapabilities) -> SendMessage {
        let channel = Arc::new(RecordingChannel {
            capabilities,
            sent: Mutex::new(None),
        });
        SendMessage::of(
            ChannelHandle::new(channel, ChatId(42)),
            host_sandbox(),
            multimodal,
        )
    }

    #[tokio::test]
    async fn a_field_stays_while_an_advertised_kind_needs_it() {
        let schema = tool_with(speaking_service(), capabilities(&ALL_FORMS, true, true))
            .schema()
            .expect("schema");

        // The voice form takes `prompt`, and this build can speak, so the argument stays.
        assert!(schema["properties"].get("prompt").is_some(), "{schema}");
    }

    #[tokio::test]
    async fn a_file_over_the_platform_ceiling_is_refused() {
        let capabilities = ChannelCapabilities {
            max_media_bytes: 4,
            ..capabilities(&ALL_FORMS, true, true)
        };
        let tool = tool_with(service(), capabilities);
        let path = std::env::temp_dir().join(format!("send-test-{}.log", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, b"a report").await.expect("write");

        let error = tool
            .execute(json!({ "kind": "file", "source": format!("file://{}", path.display()) }))
            .await
            .expect_err("over the ceiling");
        assert!(error.to_string().contains("larger than"), "{error}");

        tokio::fs::remove_file(&path).await.expect("cleanup");
    }

    #[tokio::test]
    async fn text_without_content_is_refused() {
        let (tool, _bot) = recording();
        let error = tool
            .execute(json!({ "kind": "text" }))
            .await
            .expect_err("missing content");
        assert!(error.to_string().contains("requires"), "{error}");
    }

    #[tokio::test]
    async fn a_kind_the_platform_lacks_is_refused_and_never_advertised() {
        let channel = Arc::new(RecordingChannel {
            capabilities: capabilities(&TEXT_AND_FILE, true, true),
            sent: Mutex::new(None),
        });
        let tool = tool(channel);

        let error = tool
            .execute(json!({ "kind": "voice", "prompt": "softly" }))
            .await
            .expect_err("voice is not available");
        assert!(error.to_string().contains("not available here"), "{error}");

        let schema = tool.schema().expect("schema");
        let kinds = schema["properties"]["kind"]["enum"]
            .as_array()
            .expect("kind enum")
            .clone();
        assert!(!kinds.contains(&json!("voice")), "{kinds:?}");
        assert!(kinds.contains(&json!("text")), "{kinds:?}");

        // The guide the model reads is built from the same table, so the pruned kind leaves no line.
        let guide = schema["properties"]["kind"]["description"]
            .as_str()
            .expect("kind guide")
            .to_owned();
        assert!(!guide.contains("voice"), "{guide}");
        assert!(guide.contains("`text` takes content"), "{guide}");
    }

    #[tokio::test]
    async fn the_schema_drops_what_the_platform_cannot_take() {
        let channel = Arc::new(RecordingChannel {
            capabilities: capabilities(&ALL_FORMS, false, false),
            sent: Mutex::new(None),
        });
        let schema = tool(channel).schema().expect("schema");

        assert!(schema["properties"].get("reference").is_none(), "{schema}");
        assert!(schema["properties"].get("quote").is_none(), "{schema}");
        assert!(schema["properties"].get("content").is_some(), "{schema}");
    }

    #[tokio::test]
    async fn a_field_the_platform_cannot_take_is_refused() {
        let channel = Arc::new(RecordingChannel {
            capabilities: capabilities(&ALL_FORMS, false, false),
            sent: Mutex::new(None),
        });
        let tool = tool(channel);

        let error = tool
            .execute(json!({ "kind": "text", "content": "hi", "reference": 7 }))
            .await
            .expect_err("no references on this platform");
        assert!(
            error.to_string().contains("takes no `reference`"),
            "{error}"
        );

        let error = tool
            .execute(json!({ "kind": "text", "content": "hi", "quote": "hi" }))
            .await
            .expect_err("no quotes on this platform");
        assert!(error.to_string().contains("takes no `quote`"), "{error}");
    }

    #[tokio::test]
    async fn a_field_that_belongs_to_another_kind_is_refused() {
        let (tool, _bot) = recording();

        let error = tool
            .execute(json!({ "kind": "text", "content": "hi", "source": "workspace://a.png" }))
            .await
            .expect_err("a source on text");
        assert!(error.to_string().contains("takes no `source`"), "{error}");

        let error = tool
            .execute(json!({ "kind": "file", "source": "workspace://a.png", "prompt": "a cat" }))
            .await
            .expect_err("a prompt on file");
        assert!(error.to_string().contains("takes no `prompt`"), "{error}");
    }

    #[tokio::test]
    async fn an_image_file_is_sent_as_a_photo() {
        let (tool, channel) = recording();
        let path = std::env::temp_dir().join(format!("send-test-{}.png", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, PNG_MAGIC).await.expect("write");

        tool.execute(json!({ "kind": "file", "source": format!("file://{}", path.display()) }))
            .await
            .expect("send");
        assert!(
            matches!(sent_payload(&channel), OutboundPayload::Photo { .. }),
            "expected a photo"
        );

        tokio::fs::remove_file(&path).await.expect("cleanup");
    }

    #[tokio::test]
    async fn an_unknown_file_is_sent_as_a_document() {
        let (tool, channel) = recording();
        let path = std::env::temp_dir().join(format!("send-test-{}.log", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, b"a report").await.expect("write");

        tool.execute(json!({ "kind": "file", "source": format!("file://{}", path.display()) }))
            .await
            .expect("send");
        match sent_payload(&channel) {
            OutboundPayload::Document { filename, .. } => {
                assert!(filename.ends_with(".log"), "{filename}")
            }
            other => panic!("expected a document, got {other:?}"),
        }

        tokio::fs::remove_file(&path).await.expect("cleanup");
    }

    #[tokio::test]
    async fn a_url_source_stays_a_url() {
        let (tool, channel) = recording();

        tool.execute(json!({ "kind": "file", "source": "https://example.com/a.pdf" }))
            .await
            .expect("send");
        match sent_payload(&channel) {
            OutboundPayload::Document {
                source: OutboundSource::Url(url),
                filename,
                ..
            } => {
                assert_eq!(url.as_str(), "https://example.com/a.pdf");
                assert_eq!(filename, "a.pdf");
            }
            other => panic!("expected a document carrying the URL, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_relative_source_reads_the_workspace() {
        let (workspace, _dir) = crate::agent::test_support::created_workspace().await;
        let sandbox = Arc::new(Sandbox::host(workspace, 30));
        tokio::fs::write(sandbox.host_workspace().join("a.png"), PNG_MAGIC)
            .await
            .expect("write");

        let (tool, channel) = recording_in(sandbox);
        tool.execute(json!({ "kind": "file", "source": "a.png" }))
            .await
            .expect("send");

        assert!(
            matches!(
                sent_payload(&channel),
                OutboundPayload::Photo {
                    source: OutboundSource::Bytes(_),
                    ..
                }
            ),
            "a relative source resolves inside the workspace"
        );
    }
}
