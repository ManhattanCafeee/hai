use crate::domain::{
    model::Platform,
    vo::{
        AttachmentId, AttachmentKind, AttachmentMeta, AttachmentParser, ChatId, MediaCodec,
        MessageId,
    },
};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Attachment {
    pub id: AttachmentId,

    pub message_id: MessageId,
    pub ordinal: i32,
    pub platform: Platform,
    /// The platform's stable file identity; absent for a row this build could not read.
    pub file_unique_id: Option<String>,
    /// The platform's download handle.
    pub file_id: String,

    pub kind: AttachmentKind,
    pub caption: Option<String>,
    pub meta: AttachmentMeta,
    #[sqlx(try_from = "jiff_sqlx::Timestamp")]
    pub created_at: jiff::Timestamp,
}

impl Attachment {
    /// Which sensory pipeline can read this file; `None` when nothing can.
    pub fn parser(&self) -> Option<AttachmentParser> {
        match self.kind {
            AttachmentKind::Photo | AttachmentKind::Sticker => Some(AttachmentParser::Image),
            AttachmentKind::Video | AttachmentKind::Animation | AttachmentKind::VideoNote => {
                Some(AttachmentParser::Video)
            }
            AttachmentKind::Audio | AttachmentKind::Voice => Some(AttachmentParser::Audio),
            AttachmentKind::Document => self
                .meta
                .file_name
                .as_deref()
                .and_then(AttachmentParser::from_filename),
        }
    }

    /// The codec the audio or video model has to be told about: what the platform declared with the
    /// file, else what the kind implies — the fallback for rows written before that declaration.
    pub fn media_format(&self) -> Option<MediaCodec> {
        self.meta.media_codec.or_else(|| match self.kind {
            AttachmentKind::Voice => Some(MediaCodec::Ogg),
            AttachmentKind::Audio => Some(MediaCodec::Mp3),
            AttachmentKind::Document => self
                .meta
                .file_name
                .as_deref()
                .and_then(MediaCodec::from_filename),
            _ => None,
        })
    }

    /// The perception subject for this file: the stable handle when the platform gave one.
    pub fn subject(&self) -> crate::domain::vo::Subject {
        crate::domain::vo::Subject::handle(
            self.platform.as_str(),
            self.file_unique_id.as_deref().unwrap_or(&self.file_id),
        )
    }
}

/// The payload one attachment row is written from.
#[derive(Debug, Clone)]
pub struct NewAttachment {
    pub id: AttachmentId,
    pub ordinal: i32,
    pub platform: Platform,
    pub file_unique_id: Option<String>,
    pub file_id: String,
    pub kind: AttachmentKind,
    pub caption: Option<String>,
    pub meta: AttachmentMeta,
}

impl NewAttachment {
    /// The row one message's file becomes: the id namespace, the platform spelling and the
    /// occurrence's place in its message live here and nowhere else.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        platform: Platform,
        ordinal: i32,
        chat_id: ChatId,
        external_id: &str,
        kind: AttachmentKind,
        file_unique_id: Option<String>,
        file_id: String,
        caption: Option<String>,
        meta: AttachmentMeta,
    ) -> Self {
        Self {
            id: crate::domain::vo::attachment_id(platform.as_str(), chat_id, external_id, ordinal),
            ordinal,
            platform,
            file_unique_id,
            file_id,
            kind,
            caption,
            meta,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::test_support;

    fn attachment(kind: AttachmentKind, file_name: Option<&str>) -> Attachment {
        Attachment {
            meta: AttachmentMeta {
                file_name: file_name.map(str::to_owned),
                ..Default::default()
            },
            ..test_support::attachment(kind)
        }
    }

    #[test]
    fn each_kind_picks_its_pipeline() {
        assert_eq!(
            attachment(AttachmentKind::Photo, None).parser(),
            Some(AttachmentParser::Image)
        );
        assert_eq!(
            attachment(AttachmentKind::Sticker, None).parser(),
            Some(AttachmentParser::Image)
        );
        assert_eq!(
            attachment(AttachmentKind::Video, None).parser(),
            Some(AttachmentParser::Video)
        );
        assert_eq!(
            attachment(AttachmentKind::Animation, None).parser(),
            Some(AttachmentParser::Video)
        );
        assert_eq!(
            attachment(AttachmentKind::VideoNote, None).parser(),
            Some(AttachmentParser::Video)
        );
        assert_eq!(
            attachment(AttachmentKind::Audio, None).parser(),
            Some(AttachmentParser::Audio)
        );
        assert_eq!(
            attachment(AttachmentKind::Voice, None).parser(),
            Some(AttachmentParser::Audio)
        );
        assert_eq!(
            attachment(AttachmentKind::Document, Some("clip.mp3")).parser(),
            Some(AttachmentParser::Audio)
        );
        assert_eq!(
            attachment(AttachmentKind::Document, Some("clip.aac")).parser(),
            Some(AttachmentParser::Audio)
        );
        assert_eq!(
            attachment(AttachmentKind::Document, Some("shot.jpg")).parser(),
            Some(AttachmentParser::Image)
        );
        assert_eq!(
            attachment(AttachmentKind::Document, Some("clip.mp4")).parser(),
            Some(AttachmentParser::Video)
        );
        // Nothing reads a document whose name names no media, and a nameless one is unreadable.
        assert_eq!(
            attachment(AttachmentKind::Document, Some("paper.pdf")).parser(),
            None
        );
        assert_eq!(attachment(AttachmentKind::Document, None).parser(), None);
    }

    #[test]
    fn media_format_follows_the_kind() {
        assert_eq!(
            attachment(AttachmentKind::Voice, None).media_format(),
            Some(MediaCodec::Ogg)
        );
        assert_eq!(
            attachment(AttachmentKind::Audio, None).media_format(),
            Some(MediaCodec::Mp3)
        );
        assert_eq!(
            attachment(AttachmentKind::Document, Some("clip.m4a")).media_format(),
            Some(MediaCodec::M4a)
        );
        assert_eq!(
            attachment(AttachmentKind::Document, Some("clip.aac")).media_format(),
            Some(MediaCodec::Aac)
        );
        assert_eq!(
            attachment(AttachmentKind::Document, Some("CLIP.M4A")).media_format(),
            Some(MediaCodec::M4a)
        );
        assert_eq!(attachment(AttachmentKind::Photo, None).media_format(), None);
    }

    #[test]
    fn a_subject_prefers_the_stable_handle() {
        assert_eq!(
            attachment(AttachmentKind::Photo, None).subject().value,
            "telegram:uniq"
        );
        let mut without_unique = attachment(AttachmentKind::Photo, None);
        without_unique.file_unique_id = None;
        assert_eq!(without_unique.subject().value, "telegram:handle");
    }
}
