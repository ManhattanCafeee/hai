//! One analysis pipeline: a payload, the parser that reads it, and the perception cache.

use crate::{
    agent::{
        channel::ChannelHandle,
        multimodal::{ImageSource, MediaSource, MultimodalService, Payload},
    },
    domain::{
        service::Services,
        vo::{AttachmentId, AttachmentKind, AttachmentParser, ImageFormat, Subject},
    },
    error::{AppError, Result},
};

/// The one place a payload is read: the parser is chosen here, the analysis is cached here.
#[derive(Clone)]
pub struct MediaAnalyzer {
    multimodal: MultimodalService,
    services: Services,
    chat: ChannelHandle,
    inline_limit: InlineLimit,
}

impl MediaAnalyzer {
    pub fn new(
        multimodal: MultimodalService,
        services: Services,
        chat: ChannelHandle,
        inline_limit: InlineLimit,
    ) -> Self {
        Self {
            multimodal,
            services,
            chat,
            inline_limit,
        }
    }

    /// The ceiling a caller refuses a file with before reading it.
    pub(crate) fn inline_limit(&self) -> InlineLimit {
        self.inline_limit
    }

    /// The services this analyzer reads through: one handle for the whole read tool, rather than a
    /// second clone beside it.
    pub(crate) fn services(&self) -> &Services {
        &self.services
    }

    /// Analyzes `payload` and caches the result under `subject`; no subject means no cache.
    pub async fn analyze(
        &self,
        payload: Payload,
        parser: AttachmentParser,
        parser_override: Option<AttachmentParser>,
        subject: Option<Subject>,
        focus: Option<&str>,
    ) -> Result<String> {
        let focus = focus_of(focus);
        let parser = resolve(parser, parser_override)?;
        self.check_inline_size(&payload)?;
        if let Some(subject) = &subject
            && let Some(hit) = self
                .services
                .perception
                .find(subject, parser, focus)
                .await?
        {
            return Ok(hit.content);
        }
        let content = self.multimodal.analyze(parser, payload, focus).await?;
        if let Some(subject) = &subject {
            self.services
                .perception
                .upsert(subject, parser, focus, &content)
                .await?;
        }
        Ok(content)
    }

    /// The attachment path: the row decides parser and subject, the platform the payload.
    pub async fn analyze_attachment(
        &self,
        id: AttachmentId,
        parser_override: Option<AttachmentParser>,
        focus: Option<&str>,
    ) -> Result<String> {
        let attachment = self
            .services
            .message
            .attachment_by_id(id)
            .await?
            .ok_or_else(|| AppError::message(format!("no attachment has id {id}")))?;
        let detected = attachment.parser().ok_or_else(|| {
            AppError::message(format!(
                "this file cannot be analyzed: {}",
                attachment
                    .meta
                    .file_name
                    .as_deref()
                    .unwrap_or(&attachment.file_id)
            ))
        })?;
        let parser = resolve(detected, parser_override)?;
        let payload = match (parser, attachment.kind) {
            (AttachmentParser::Audio, _) => Payload::Media(
                MediaSource::Bytes(self.chat.download_file(&attachment.file_id).await?),
                attachment.media_format(),
            ),
            (AttachmentParser::Video, _) => Payload::Media(
                MediaSource::Url(self.chat.file_url(&attachment.file_id).await?),
                attachment.media_format(),
            ),
            (AttachmentParser::Image | AttachmentParser::Ocr, AttachmentKind::Sticker) => {
                let data = self.chat.download_file(&attachment.file_id).await?;
                let format = ImageFormat::sniff(&data).ok_or_else(|| {
                    AppError::message("sticker is not an image the vision model can read")
                })?;
                Payload::Image(ImageSource::bytes(data, format))
            }
            (AttachmentParser::Image | AttachmentParser::Ocr, _) => Payload::Image(
                ImageSource::Url(self.chat.file_url(&attachment.file_id).await?),
            ),
        };
        self.analyze(payload, parser, None, Some(attachment.subject()), focus)
            .await
    }

    /// Bytes handed to a model are bounded here, so no caller carries the limit.
    fn check_inline_size(&self, payload: &Payload) -> Result<()> {
        let len = match payload {
            Payload::Image(ImageSource::Bytes { data, .. }) => data.len(),
            Payload::Media(MediaSource::Bytes(data), _) => data.len(),
            Payload::Image(ImageSource::Url(_)) | Payload::Media(MediaSource::Url(_), _) => {
                return Ok(());
            }
        };
        self.inline_limit.refuse(len as u64)
    }
}

/// An empty instruction is no instruction: the cache key and the prompt both treat it as absent.
fn focus_of(focus: Option<&str>) -> Option<&str> {
    focus.map(str::trim).filter(|focus| !focus.is_empty())
}

/// The inline ceiling one request may carry; `0` leaves it unbounded.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InlineLimit(u64);

impl InlineLimit {
    pub(crate) fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    /// Refuses bytes over the limit before they are read or sent.
    pub(crate) fn refuse(&self, len: u64) -> Result<(), AppError> {
        if self.0 != 0 && len > self.0 {
            return Err(AppError::message(format!(
                "inline analysis is limited to {} MB",
                self.0 / (1024 * 1024)
            )));
        }
        Ok(())
    }
}

/// The requested parser wins when it is another reading of the same payload.
fn resolve(
    detected: AttachmentParser,
    requested: Option<AttachmentParser>,
) -> Result<AttachmentParser> {
    match requested {
        None => Ok(detected),
        Some(AttachmentParser::Ocr) if detected == AttachmentParser::Image => {
            Ok(AttachmentParser::Ocr)
        }
        Some(p) if p == detected => Ok(p),
        Some(p) => Err(AppError::message(format!(
            "parser `{p}` cannot read this file"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_switches_between_the_two_readings_of_an_image() {
        assert_eq!(
            resolve(AttachmentParser::Image, None).unwrap(),
            AttachmentParser::Image
        );
        assert_eq!(
            resolve(AttachmentParser::Image, Some(AttachmentParser::Ocr)).unwrap(),
            AttachmentParser::Ocr
        );
        assert_eq!(
            resolve(AttachmentParser::Audio, Some(AttachmentParser::Audio)).unwrap(),
            AttachmentParser::Audio
        );
    }

    #[test]
    fn an_override_cannot_move_a_file_to_another_pipeline() {
        assert!(resolve(AttachmentParser::Image, Some(AttachmentParser::Video)).is_err());
        assert!(resolve(AttachmentParser::Video, Some(AttachmentParser::Audio)).is_err());
        assert!(resolve(AttachmentParser::Audio, Some(AttachmentParser::Image)).is_err());
    }

    #[test]
    fn an_oversized_inline_payload_is_refused() {
        const MAX: u64 = 20 * 1024 * 1024;
        assert!(InlineLimit::new(MAX).refuse(16).is_ok());
        assert!(
            InlineLimit::new(0).refuse(MAX + 1).is_ok(),
            "0 is unbounded"
        );
        let error = InlineLimit::new(MAX).refuse(MAX + 1).expect_err("refused");
        assert!(error.to_string().contains("20 MB"), "{error}");
    }

    #[test]
    fn an_empty_focus_is_no_focus() {
        assert_eq!(focus_of(None), None);
        assert_eq!(focus_of(Some("")), None);
        assert_eq!(focus_of(Some("  ")), None);
        assert_eq!(focus_of(Some(" x ")), Some("x"));
    }
}
