use std::sync::Arc;

use base64::{Engine as _, encoded_len, prelude::BASE64_STANDARD};
use serde_json::{Value, json};

use crate::{
    agentcore::{ApiClient, ModelKind, ModelTarget},
    config::{AppConfig, ConfigIssue, model_role::ModelRoles, schema::SpeechConfig},
    domain::vo::{AttachmentParser, ImageFormat, MediaCodec},
    error::{AppError, Result, bail},
};

pub(crate) mod analysis;
mod prompts;

use self::prompts::{
    AUDIO_JUDGMENT_PROMPT, DEFAULT_AUDIO_PROMPT, DEFAULT_IMAGE_PROMPT, DEFAULT_OCR_PROMPT,
    DEFAULT_VIDEO_PROMPT, IMAGE_JUDGMENT_PROMPT, VIDEO_JUDGMENT_PROMPT, build_prompt,
};

pub enum ImageSource {
    Url(String),
    Bytes { data: Vec<u8>, format: ImageFormat },
}

impl ImageSource {
    pub fn bytes(data: Vec<u8>, format: ImageFormat) -> Self {
        Self::Bytes { data, format }
    }

    fn into_image_url(self) -> String {
        match self {
            Self::Url(url) => url,
            Self::Bytes { data, format } => {
                let mime = format.mime();
                let mut url = String::with_capacity(
                    "data:;base64,".len() + mime.len() + encoded_len(data.len(), true).unwrap_or(0),
                );
                url.push_str("data:");
                url.push_str(mime);
                url.push_str(";base64,");
                BASE64_STANDARD.encode_string(data, &mut url);
                url
            }
        }
    }
}

/// Media handed to the video or audio model; the caller supplies the codec with each call.
pub enum MediaSource {
    Url(String),
    Bytes(Vec<u8>),
}

impl MediaSource {
    fn into_api_value(self, fmt: &str) -> Value {
        match self {
            MediaSource::Url(u) => json!({ "url": u }),
            MediaSource::Bytes(d) => {
                json!({ "data": BASE64_STANDARD.encode(d), "format": fmt })
            }
        }
    }
}

pub enum Payload {
    Image(ImageSource),
    Media(MediaSource, Option<MediaCodec>),
}

/// A model kind HAI can read, with the template its transcription falls back to.
#[derive(Debug)]
struct AnalysisCfg {
    target: ModelTarget,
    default_prompt: String,
}

#[derive(Debug)]
struct SpeechCfg {
    target: ModelTarget,
    voice: String,
    speed: f32,
}

#[derive(Debug)]
struct MultimodalServiceInner {
    client: ApiClient,
    vision: Option<AnalysisCfg>,
    video: Option<AnalysisCfg>,
    transcription: Option<AnalysisCfg>,
    speech: Option<SpeechCfg>,
    image: Option<ModelTarget>,
}

#[derive(Debug, Clone)]
pub struct MultimodalService(Arc<MultimodalServiceInner>);

impl MultimodalService {
    /// Interprets the roles the table resolved: each one is a target, plus the `[agent.multimodal]`
    /// switch and template around it.
    pub(crate) fn from_config(config: &AppConfig, roles: &ModelRoles) -> Result<Self> {
        let analysis = &config.agent.multimodal;
        let speech_block = config.model_roles.speech.as_ref();

        Ok(Self(Arc::new(MultimodalServiceInner {
            client: ApiClient::try_new()?,
            vision: analysis_cfg(
                roles.vision.as_ref(),
                analysis.vision.enabled,
                analysis.vision.image_prompt.as_deref(),
                DEFAULT_IMAGE_PROMPT,
            ),
            video: analysis_cfg(
                roles.vision.as_ref(),
                analysis.vision.enabled,
                analysis.vision.video_prompt.as_deref(),
                DEFAULT_VIDEO_PROMPT,
            ),
            transcription: analysis_cfg(
                roles.transcription.as_ref(),
                analysis.transcription.enabled,
                analysis.transcription.prompt.as_deref(),
                DEFAULT_AUDIO_PROMPT,
            ),
            speech: speech_cfg(roles.speech.as_ref(), speech_block)?,
            image: roles.image.clone(),
        })))
    }

    pub async fn analyze_image(&self, source: ImageSource, focus: Option<&str>) -> Result<String> {
        let cfg = self
            .0
            .vision
            .as_ref()
            .ok_or_else(|| unavailable("image analysis"))?;
        let text = build_prompt(IMAGE_JUDGMENT_PROMPT, &cfg.default_prompt, focus);
        let body = json!([
            {"type": "text", "text": text},
            {"type": "image_url", "image_url": {"url": source.into_image_url()}},
        ]);
        self.complete_and_extract(&cfg.target, body, "vision").await
    }

    pub async fn ocr(&self, source: ImageSource) -> Result<String> {
        let cfg = self
            .0
            .vision
            .as_ref()
            .ok_or_else(|| unavailable("image analysis"))?;
        let body = json!([
            {"type": "text", "text": DEFAULT_OCR_PROMPT},
            {"type": "image_url", "image_url": {"url": source.into_image_url()}},
        ]);
        self.complete_and_extract(&cfg.target, body, "vision").await
    }

    pub async fn analyze_video(
        &self,
        source: MediaSource,
        format: Option<MediaCodec>,
        focus: Option<&str>,
    ) -> Result<String> {
        let cfg = self
            .0
            .video
            .as_ref()
            .ok_or_else(|| unavailable("video analysis"))?;
        let text = build_prompt(VIDEO_JUDGMENT_PROMPT, &cfg.default_prompt, focus);
        let fmt = format.as_ref().map(|c| c.api_format()).unwrap_or("mp4");
        let body = json!([
            {"type": "text", "text": text},
            {"type": "video_url", "video_url": source.into_api_value(fmt)},
        ]);
        self.complete_and_extract(&cfg.target, body, "video").await
    }

    pub async fn analyze_audio(
        &self,
        source: MediaSource,
        format: Option<MediaCodec>,
        focus: Option<&str>,
    ) -> Result<String> {
        let cfg = self
            .0
            .transcription
            .as_ref()
            .ok_or_else(|| unavailable("audio analysis"))?;
        let text = build_prompt(AUDIO_JUDGMENT_PROMPT, &cfg.default_prompt, focus);
        let fmt = format.as_ref().map(|c| c.api_format()).unwrap_or("wav");
        let body = json!([
            {"type": "text", "text": text},
            {"type": "input_audio", "input_audio": source.into_api_value(fmt)},
        ]);
        self.complete_and_extract(&cfg.target, body, "audio").await
    }

    /// Runs `parser` on `payload`; the two must belong to the same payload family.
    pub async fn analyze(
        &self,
        parser: AttachmentParser,
        payload: Payload,
        focus: Option<&str>,
    ) -> Result<String> {
        match (parser, payload) {
            (AttachmentParser::Image, Payload::Image(source)) => {
                self.analyze_image(source, focus).await
            }
            (AttachmentParser::Ocr, Payload::Image(source)) => self.ocr(source).await,
            (AttachmentParser::Video, Payload::Media(source, codec)) => {
                self.analyze_video(source, codec, focus).await
            }
            (AttachmentParser::Audio, Payload::Media(source, codec)) => {
                self.analyze_audio(source, codec, focus).await
            }
            (parser, _) => Err(AppError::message(format!(
                "parser {parser} does not read this payload"
            ))),
        }
    }

    pub fn vision_enabled(&self) -> bool {
        self.0.vision.is_some()
    }

    pub fn transcription_enabled(&self) -> bool {
        self.0.transcription.is_some()
    }

    pub fn image_enabled(&self) -> bool {
        self.0.image.is_some()
    }

    pub fn speech_enabled(&self) -> bool {
        self.0.speech.is_some()
    }

    pub async fn generate_image(
        &self,
        prompt: &str,
        images: &[crate::agentcore::api::ImageUrl],
    ) -> Result<Vec<u8>> {
        let ep = self
            .0
            .image
            .as_ref()
            .ok_or_else(|| unavailable("image generation"))?;
        self.0.client.generate_image(ep, prompt, images).await
    }

    pub async fn speech(&self, text: &str) -> Result<Vec<u8>> {
        let cfg = self
            .0
            .speech
            .as_ref()
            .ok_or_else(|| unavailable("speech"))?;
        self.0
            .client
            .speech(&cfg.target, text, &cfg.voice, cfg.speed)
            .await
    }

    async fn complete_and_extract(
        &self,
        target: &ModelTarget,
        body: Value,
        tag: &str,
    ) -> Result<String> {
        let response = self.0.client.complete(target, body).await?;
        response
            .text()
            .map(str::to_owned)
            .ok_or_else(|| AppError::message(format!("Failed to parse {tag} response")))
    }
}

/// The analysis config of one understanding role: the target the table resolved, the switch and the
/// template. A switched-off role mounts nothing.
fn analysis_cfg(
    target: Option<&ModelTarget>,
    enabled: bool,
    prompt: Option<&str>,
    default_prompt: &str,
) -> Option<AnalysisCfg> {
    if !enabled {
        return None;
    }
    target.map(|target| AnalysisCfg {
        target: target.clone(),
        default_prompt: prompt.unwrap_or(default_prompt).into(),
    })
}

/// The speech config: the target the table resolved plus the block's own voice and speed.
fn speech_cfg(
    target: Option<&ModelTarget>,
    block: Option<&SpeechConfig>,
) -> Result<Option<SpeechCfg>> {
    let (Some(target), Some(block)) = (target, block) else {
        return Ok(None);
    };
    if block.voice.is_empty() {
        bail!(ConfigIssue::model_role(
            ModelKind::Speech,
            "voice is required (voice names are model-scoped)"
        ));
    }
    Ok(Some(SpeechCfg {
        target: target.clone(),
        voice: block.voice.clone(),
        speed: block.speed,
    }))
}

fn unavailable(name: &str) -> crate::error::AppError {
    AppError::message(format!("{name} is unavailable"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ProviderRegistry};

    #[test]
    fn image_bytes_carry_their_own_format_into_the_data_url() {
        let png = ImageSource::bytes(b"\x89PNG".to_vec(), ImageFormat::Png);
        assert_eq!(png.into_image_url(), "data:image/png;base64,iVBORw==");

        let webp = ImageSource::bytes(vec![0x00, 0x01, 0x02], ImageFormat::Webp);
        assert_eq!(webp.into_image_url(), "data:image/webp;base64,AAEC");

        let url = ImageSource::Url("https://example.com/a.webp".to_owned());
        assert_eq!(url.into_image_url(), "https://example.com/a.webp");
    }

    /// The analysis switch and the model block are separate: a `[model-roles.vision]` block carries only
    /// the model, and `[agent.multimodal.vision]` is enabled unless turned off.
    #[test]
    fn the_vision_analysis_defaults_on() {
        let vision: crate::config::schema::VisionConfig =
            toml::from_str("model = \"gpt-4o\"").unwrap();
        assert_eq!(vision.model.as_deref(), Some("gpt-4o"));

        let analysis = crate::config::schema::VisionAnalysisConfig::default();
        assert!(analysis.enabled);
        assert!(analysis.image_prompt.is_none());
    }

    /// The roles these tests exercise are about resolution, not availability, so every provider is
    /// given a credential and every endpoint answers.
    fn registry(cfg: &AppConfig) -> ProviderRegistry {
        ProviderRegistry::from_sources(cfg, &|_| Some("test-key".to_owned()), &|_| true)
    }

    /// The service one config builds: the roles resolved, then interpreted.
    fn service(cfg: &AppConfig) -> Result<MultimodalService> {
        let roles = crate::config::model_role::resolve(cfg, &registry(cfg))?;
        MultimodalService::from_config(cfg, &roles)
    }

    /// The config every case starts from: one chat model with its provider registered, plus the
    /// embedding role the service insists on.
    fn config(provider: &str, model: &str) -> AppConfig {
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

    /// An absent understanding block leaves the capability off when the main provider cannot read
    /// that media, instead of failing startup over a block nobody configured.
    #[tokio::test]
    async fn an_absent_block_on_a_provider_that_cannot_read_the_media_stays_off() {
        let cfg = config("anthropic", "claude-sonnet-4-5");

        let svc = service(&cfg).unwrap();
        assert!(
            svc.analyze_image(ImageSource::Url("https://example.com/a.png".into()), None)
                .await
                .is_err()
        );
    }

    /// An absent understanding block runs on the main model and inherits its endpoint, so a proxied
    /// `[model-roles.default]` does not send the delegated call to the provider's own root.
    #[test]
    fn an_absent_understanding_block_inherits_the_default_endpoint() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.model_roles.default.base_url = Some("https://gateway.internal/v1".into());

        let svc = service(&cfg).unwrap();
        assert!(svc.vision_enabled());
        assert!(svc.transcription_enabled());

        let vision = svc.0.vision.as_ref().unwrap();
        assert_eq!(vision.target.base_url, "https://gateway.internal/v1");
        assert_eq!(vision.target.model, "gpt-4o");
    }

    /// A block that declares a vision model on a provider which cannot read images fails startup.
    #[test]
    fn a_declared_understanding_block_is_checked() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.model_roles.vision = Some(crate::config::schema::VisionConfig {
            provider: Some("anthropic".into()),
            model: Some("claude-sonnet-4-5".into()),
            base_url: None,
        });
        cfg.providers.insert("anthropic".into(), Default::default());

        let err = service(&cfg).unwrap_err();
        assert!(err.to_string().contains("serves no vision API"), "{err}");
    }

    /// A configured `[model-roles.image]` resolves to an image role in `from_config`.
    #[test]
    fn image_from_config() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.model_roles.image = Some(crate::config::schema::ImageConfig {
            provider: Some("openrouter".into()),
            model: Some("google/gemini-2.5-flash-image".into()),
            base_url: None,
        });
        cfg.providers
            .insert("openrouter".into(), Default::default());
        let svc = service(&cfg).unwrap();
        assert!(svc.image_enabled());
    }

    /// Conditional mounting: an absent `[model-roles.image]` leaves image generation disabled.
    #[test]
    fn image_disabled_when_unconfigured() {
        let cfg = config("openai", "gpt-4o");
        let svc = service(&cfg).unwrap();
        assert!(!svc.image_enabled());
    }

    /// Conditional mounting: a configured `[model-roles.image]` with an empty model stays disabled.
    #[test]
    fn image_disabled_when_model_empty() {
        let mut cfg = config("openai", "gpt-4o");
        cfg.model_roles.image = Some(crate::config::schema::ImageConfig {
            provider: Some("openai".into()),
            model: None,
            base_url: None,
        });
        let svc = service(&cfg).unwrap();
        assert!(!svc.image_enabled());
    }

    /// An omitted provider falls back to `model_roles.default.provider` when image generation configures
    /// only a model.
    #[test]
    fn image_provider_falls_back_to_agent() {
        let mut cfg = config("openrouter", "gpt-4o");
        cfg.model_roles.image = Some(crate::config::schema::ImageConfig {
            provider: None, // omitted → model_roles.default.provider
            model: Some("google/gemini-2.5-flash-image".into()),
            base_url: None,
        });
        let svc = service(&cfg).unwrap();
        assert!(svc.image_enabled());
    }

    fn speech_config(voice: &str, model: Option<&str>) -> AppConfig {
        let mut cfg = config("openai", "gpt-4o");
        cfg.model_roles.speech = Some(crate::config::schema::SpeechConfig {
            provider: None,
            model: model.map(str::to_owned),
            voice: voice.into(),
            speed: 1.0,
            base_url: None,
        });
        cfg
    }

    /// A `[model-roles.speech]` block with model and voice makes `send_message`'s voice kind available.
    #[test]
    fn speech_enabled_when_configured() {
        let cfg = speech_config("alloy", Some("gpt-4o-mini-tts"));
        let svc = service(&cfg).unwrap();
        assert!(svc.speech_enabled());
    }

    /// Conditional mounting: a configured `[model-roles.speech]` with an empty model stays disabled.
    #[test]
    fn speech_disabled_when_model_empty() {
        let cfg = speech_config("alloy", None);
        let svc = service(&cfg).unwrap();
        assert!(!svc.speech_enabled());
    }

    /// A voice name has no generic default: a configured model with an empty voice is a contract failure,
    /// not a runtime provider 400.
    #[test]
    fn speech_voice_missing_is_config_error() {
        let cfg = speech_config("", Some("gpt-4o-mini-tts"));
        assert!(service(&cfg).is_err());
    }
}
