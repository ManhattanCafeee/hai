//! One accepted send, read into the receipt the store records: the platform's own id and time, what
//! the reference did, and one attachment per file that went out.

use crate::{
    domain::vo::{AttachmentKind, AttachmentMeta},
    error::Result,
    platform::{
        adapter::{SentAttachment, SentMessage, WireReference},
        telegram::reply,
    },
};

/// The receipt one accepted send becomes. Every send arm goes through here, so the id, the time and
/// the reference's own account of itself are read once, in one shape.
pub(super) fn receipt(
    sent: &teloxide::types::Message,
    reference: Option<&WireReference>,
    attachments: Vec<SentAttachment>,
) -> Result<SentMessage> {
    Ok(SentMessage {
        external_id: sent.id.to_string(),
        sent_at: jiff::Timestamp::from_second(sent.date.timestamp())?,
        // What the platform made of it, not what was asked for: a reply the platform dropped leaves
        // no link, and recording one would put a reply edge in the store that never existed.
        reference_sent: sent.reply_to_message().is_some(),
        // Only an excerpt leaves a platform extra behind: a bare reference says what the reply
        // preview already shows.
        platform_meta: reference
            .and_then(|reference| reference.excerpt.clone())
            .map(reply::outgoing_quote_meta),
        attachments,
    })
}

/// Which media one accepted send produced: the kind and the platform's own handles.
pub(super) enum SentMedia {
    Voice,
    Photo,
    Document,
}

impl SentMedia {
    fn kind(&self) -> AttachmentKind {
        match self {
            Self::Voice => AttachmentKind::Voice,
            Self::Photo => AttachmentKind::Photo,
            Self::Document => AttachmentKind::Document,
        }
    }

    /// The platform's stable and download handles, when the response carried the file at all.
    fn handles(&self, sent_msg: &teloxide::types::Message) -> (Option<String>, Option<String>) {
        match self {
            Self::Voice => sent_msg
                .voice()
                .map(|v| (Some(v.file.unique_id.0.clone()), Some(v.file.id.0.clone())))
                .unwrap_or_default(),
            Self::Photo => sent_msg
                .photo()
                .and_then(|p| p.first())
                .map(|sz| {
                    (
                        Some(sz.file.unique_id.0.clone()),
                        Some(sz.file.id.0.clone()),
                    )
                })
                .unwrap_or_default(),
            Self::Document => sent_msg
                .document()
                .map(|d| (Some(d.file.unique_id.0.clone()), Some(d.file.id.0.clone())))
                .unwrap_or_default(),
        }
    }
}

/// What one accepted media send produced, as the store's row input.
pub(super) fn sent_attachment(
    media: SentMedia,
    sent_msg: &teloxide::types::Message,
    content: Option<&str>,
    prompt: Option<String>,
) -> SentAttachment {
    let (file_unique_id, file_id) = media.handles(sent_msg);
    SentAttachment {
        kind: media.kind(),
        file_unique_id,
        file_id,
        caption: content.map(str::to_owned),
        meta: AttachmentMeta {
            prompt,
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::domain::vo::PlatformMessageMeta;

    /// A message in the shape the Bot API answers a send with: the id, the date, and whatever media
    /// the send carried.
    fn sent_message(media: Value) -> teloxide::types::Message {
        let mut message = json!({
            "message_id": 77,
            "date": 1_700_000_000,
            "chat": { "id": 1, "type": "private" },
        });
        let base = message.as_object_mut().expect("object literal");
        for (key, value) in media.as_object().expect("object literal") {
            base.insert(key.clone(), value.clone());
        }
        serde_json::from_value(message).expect("teloxide message fixture")
    }

    /// Every arm builds its receipt here, so what the platform reported — id, time, and whether the
    /// reference actually went out — is read in one place.
    #[test]
    fn a_receipt_carries_the_platform_id_time_and_reference() {
        let answered_a_reply = sent_message(json!({
            "text": "here you go",
            "reply_to_message": {
                "message_id": 5,
                "date": 1_699_999_999,
                "chat": { "id": 1, "type": "private" },
                "from": { "id": 7, "is_bot": false, "first_name": "someone" },
            },
        }));
        let quoted = WireReference {
            external_id: "5".to_owned(),
            excerpt: Some("a part".to_owned()),
        };
        let with = receipt(&answered_a_reply, Some(&quoted), Vec::new()).expect("a receipt");

        assert_eq!(with.external_id, answered_a_reply.id.to_string());
        assert_eq!(
            with.sent_at,
            jiff::Timestamp::from_second(1_700_000_000).expect("a valid second")
        );
        assert!(with.reference_sent);
        assert!(
            matches!(with.platform_meta, Some(PlatformMessageMeta::Telegram(_))),
            "{:?}",
            with.platform_meta
        );
        assert!(with.attachments.is_empty());

        // A reference without an excerpt leaves the platform's own extra alone.
        let bare = WireReference {
            external_id: "5".to_owned(),
            excerpt: None,
        };
        let unquoted = receipt(&answered_a_reply, Some(&bare), Vec::new()).expect("a receipt");
        assert!(unquoted.reference_sent);
        assert!(unquoted.platform_meta.is_none());
    }

    /// The receipt reports what the platform did, not what was asked for: a reference the platform
    /// dropped records none, so the store never holds a reply edge that does not exist.
    #[test]
    fn a_reference_the_platform_dropped_is_not_recorded() {
        let bare = WireReference {
            external_id: "5".to_owned(),
            excerpt: None,
        };
        let sent = sent_message(json!({ "text": "here you go" }));
        let dropped = receipt(&sent, Some(&bare), Vec::new()).expect("a receipt");

        assert!(!dropped.reference_sent);
        assert!(dropped.platform_meta.is_none());
    }

    #[test]
    fn a_sent_attachment_names_its_kind_handles_and_caption() {
        let voice = sent_message(json!({
            "voice": { "file_id": "handle", "file_unique_id": "uniq", "duration": 1, "mime_type": "audio/ogg" },
        }));
        let row = sent_attachment(
            SentMedia::Voice,
            &voice,
            Some("the line"),
            Some("say it warmly".to_owned()),
        );
        assert_eq!(row.kind, AttachmentKind::Voice);
        assert_eq!(row.file_unique_id.as_deref(), Some("uniq"));
        assert_eq!(row.file_id.as_deref(), Some("handle"));
        assert_eq!(row.caption.as_deref(), Some("the line"));
        assert_eq!(row.meta.prompt.as_deref(), Some("say it warmly"));
    }

    #[test]
    fn a_response_without_the_file_reports_no_handle() {
        let bare = sent_message(json!({}));

        let photo = sent_attachment(SentMedia::Photo, &bare, None, None);
        assert_eq!(photo.kind, AttachmentKind::Photo);
        assert!(photo.file_id.is_none());
        assert!(photo.file_unique_id.is_none());
        assert!(photo.caption.is_none());

        let document = sent_attachment(SentMedia::Document, &bare, None, None);
        assert_eq!(document.kind, AttachmentKind::Document);
        assert!(document.file_id.is_none());
    }
}
