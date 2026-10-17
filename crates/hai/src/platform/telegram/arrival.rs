//! One Telegram update read into the neutral arrival the tail consumes: the chat, the sender,
//! the text and its attachments, the ids a reply points at, and who the message was aimed at. What
//! the wire's own update means — which kind of change it is, which chat type it names, whether it
//! was aimed at the bot — is read here too, so nothing platform-shaped leaves this file.

use teloxide::{
    prelude::*,
    types::{MessageEntity, MessageEntityKind},
};

use super::command::{self, CommandOwner};
use crate::{
    agent::intake::{Arrival, ArrivalChat, ArrivalSender, ArrivalText},
    domain::{
        model::{ChatType, NewAttachment, Platform},
        repo::ChangeKind,
        vo::{
            AttachmentKind, AttachmentMeta, ChatId, EntryAudience, MediaCodec, MessageMeta,
            PlatformAccountMeta, PlatformMessageMeta, TelegramAccountMeta, TelegramMessageMeta,
            TelegramQuote,
        },
    },
    error::{AppError, Result},
};

/// One incoming Telegram message read into the shape the tail consumes. `None` is a slash message
/// addressed to another bot: Telegram never delivers messages between bots, so it is conversation
/// whose other half we can never see.
pub(super) fn arrival(
    msg: &Message,
    bot: &BotIdentity<'_>,
    change: ChangeKind,
) -> Result<Option<Arrival>> {
    let chat_id = ChatId(msg.chat.id.0);
    let external_id = msg.id.to_string();
    let chat_type = msg_chat_type(msg);

    let Some(text_kind) = text_kind(msg, bot) else {
        return Ok(None);
    };
    // An edit is a change to a message, never an invocation: a command read off edited text would run
    // again on every edit of it, and a command is stored nowhere to be run twice from.
    let text_kind = match change {
        ChangeKind::Created { .. } => text_kind,
        ChangeKind::Edited { .. } => ArrivalText::Ordinary,
    };

    let attachments = match media_of(msg, msg.caption().map(str::to_owned)) {
        Some((kind, file, caption, meta)) => vec![attachment_of(
            chat_id,
            &external_id,
            kind,
            file,
            caption,
            meta,
        )],
        None => Vec::new(),
    };

    let sender = match msg.from.as_ref() {
        Some(from) => Some(ArrivalSender {
            external_id: from.id.to_string(),
            meta: Some(serde_json::to_value(PlatformAccountMeta::Telegram(
                TelegramAccountMeta {
                    first_name: from.first_name.clone(),
                    last_name: from.last_name.clone(),
                    username: from.username.clone(),
                },
            ))?),
        }),
        None => None,
    };

    let meta = MessageMeta {
        platform: Some(PlatformMessageMeta::Telegram(TelegramMessageMeta {
            message_thread_id: msg.thread_id.map(|id| id.0.0),
            quote: msg.quote().map(|quote| TelegramQuote {
                text: quote.text.clone(),
                position: quote.position,
                is_manual: quote.is_manual,
            }),
        })),
        llm: None,
    };

    Ok(Some(Arrival {
        platform: Platform::Telegram,
        chat: ArrivalChat {
            external_id: msg.chat.id.to_string(),
            kind: chat_type,
            title: msg.chat.title().map(str::to_owned),
        },
        sender,
        text: msg.text().unwrap_or_default().to_owned(),
        attachments,
        external_id,
        reply_external_id: msg.reply_to_message().map(|reply| reply.id.0.to_string()),
        change,
        sent_at: Some(jiff::Timestamp::from_second(msg.date.timestamp())?),
        audience: audience_of(msg, chat_type, bot),
        meta: serde_json::to_value(meta)?,
        text_kind,
    }))
}

/// What the message's text is: an ordinary message, the platform's own greeting, or a command of
/// ours. The spelling is Telegram's — the slash, the `@name` suffix, the payload's own spacing.
fn text_kind(msg: &Message, bot: &BotIdentity<'_>) -> Option<ArrivalText> {
    let Some(text) = msg.text() else {
        return Some(ArrivalText::Ordinary);
    };
    match command::classify(text, bot.username) {
        CommandOwner::NotACommand => Some(ArrivalText::Ordinary),
        CommandOwner::Theirs => None,
        // Telegram's START button — and the payload a deep link carries — is how a stranger first
        // touches a bot: the platform's own convention, answered and never stored.
        CommandOwner::Ours { name: "start", .. } => Some(ArrivalText::Greeting),
        CommandOwner::Ours { name, payload } => Some(ArrivalText::Command {
            name: name.to_owned(),
            payload: payload.to_owned(),
        }),
    }
}

/// The one attachment a Telegram message carries, when it carries one: the kind, the platform's file,
/// the caption that belongs to it (a sticker and a video note carry none) and whatever the kind adds.
fn media_of(
    msg: &Message,
    caption: Option<String>,
) -> Option<(
    AttachmentKind,
    &teloxide::types::FileMeta,
    Option<String>,
    AttachmentMeta,
)> {
    if let Some(photo) = msg.photo().and_then(|photos| photos.last()) {
        return Some((
            AttachmentKind::Photo,
            &photo.file,
            caption,
            AttachmentMeta::default(),
        ));
    }
    if let Some(video) = msg.video() {
        return Some((
            AttachmentKind::Video,
            &video.file,
            caption,
            AttachmentMeta {
                media_codec: codec_of(video.mime_type.as_ref().map(|mime| mime.subtype().as_str())),
                ..Default::default()
            },
        ));
    }
    if let Some(audio) = msg.audio() {
        return Some((
            AttachmentKind::Audio,
            &audio.file,
            caption,
            AttachmentMeta {
                media_codec: codec_of(audio.mime_type.as_ref().map(|mime| mime.subtype().as_str())),
                ..Default::default()
            },
        ));
    }
    if let Some(voice) = msg.voice() {
        return Some((
            AttachmentKind::Voice,
            &voice.file,
            caption,
            AttachmentMeta {
                media_codec: codec_of(voice.mime_type.as_ref().map(|mime| mime.subtype().as_str())),
                ..Default::default()
            },
        ));
    }
    if let Some(document) = msg.document() {
        return Some((
            AttachmentKind::Document,
            &document.file,
            caption,
            AttachmentMeta {
                file_name: document.file_name.clone(),
                media_codec: codec_of(
                    document
                        .mime_type
                        .as_ref()
                        .map(|mime| mime.subtype().as_str()),
                ),
                ..Default::default()
            },
        ));
    }
    if let Some(sticker) = msg.sticker() {
        return Some((
            AttachmentKind::Sticker,
            &sticker.file,
            None,
            AttachmentMeta {
                hint: sticker.emoji.clone(),
                ..Default::default()
            },
        ));
    }
    if let Some(animation) = msg.animation() {
        return Some((
            AttachmentKind::Animation,
            &animation.file,
            caption,
            AttachmentMeta {
                media_codec: codec_of(
                    animation
                        .mime_type
                        .as_ref()
                        .map(|mime| mime.subtype().as_str()),
                ),
                ..Default::default()
            },
        ));
    }
    if let Some(video_note) = msg.video_note() {
        return Some((
            AttachmentKind::VideoNote,
            &video_note.file,
            None,
            AttachmentMeta::default(),
        ));
    }
    None
}

/// The codec the wire's own media type declares; the kinds Telegram gives no `mime_type` read as
/// none, and the reader falls back to the kind.
fn codec_of(subtype: Option<&str>) -> Option<MediaCodec> {
    subtype.and_then(MediaCodec::from_mime)
}

/// One attachment row: the platform's two handles, the kind, and whatever the kind carries.
/// A Telegram message carries at most one attachment, so the ordinal is always 0.
fn attachment_of(
    chat_id: ChatId,
    external_id: &str,
    kind: AttachmentKind,
    file: &teloxide::types::FileMeta,
    caption: Option<String>,
    meta: AttachmentMeta,
) -> NewAttachment {
    NewAttachment::new(
        Platform::Telegram,
        0,
        chat_id,
        external_id,
        kind,
        Some(file.unique_id.0.clone()),
        file.id.0.clone(),
        caption,
        meta,
    )
}

/// The bot's own identity as the wire reports it: what reading one message needs to know.
pub(super) struct BotIdentity<'a> {
    pub username: &'a str,
    pub user_id: teloxide::types::UserId,
}

/// Every non-private chat — group, supergroup or channel — maps to `Group`.
pub(super) fn msg_chat_type(msg: &Message) -> ChatType {
    if msg.chat.is_private() {
        ChatType::Private
    } else {
        ChatType::Group
    }
}

/// Who a change was aimed at: a private chat, or a message that names or answers the bot.
pub(super) fn audience_of(
    msg: &Message,
    chat_type: ChatType,
    bot: &BotIdentity<'_>,
) -> EntryAudience {
    if chat_type == ChatType::Private
        || is_mentioning_user(msg, bot.username)
        || is_reply_to_bot(msg, bot.user_id)
    {
        EntryAudience::Agent
    } else {
        EntryAudience::Bystanders
    }
}

/// What one update is: which filter matched says whether it is an edit, so it is never inferred from
/// the content, and the platform's own edit time has to be there for one.
pub(super) fn change_kind(is_edit: bool, edited_at: Option<jiff::Timestamp>) -> Result<ChangeKind> {
    match (is_edit, edited_at) {
        (false, edited_at) => Ok(ChangeKind::Created { edited_at }),
        (true, Some(edited_at)) => Ok(ChangeKind::Edited { edited_at }),
        (true, None) => Err(AppError::message(
            "the platform reported an edit without an edit time",
        )),
    }
}

/// A `Mention` entity (`@username`) in the text or in the caption; a
/// `text_mention` link — a user named without `@` — never counts.
fn is_mentioning_user(msg: &Message, username: &str) -> bool {
    let username = format!("@{}", username);

    if let Some(entities) = msg.entities()
        && let Some(text) = msg.text()
        && check_entities(entities, text, &username)
    {
        return true;
    }

    if let Some(entities) = msg.caption_entities()
        && let Some(caption) = msg.caption()
        && check_entities(entities, caption, &username)
    {
        return true;
    }

    false
}

fn check_entities(entities: &[MessageEntity], text: &str, username: &str) -> bool {
    // Telegram measures an entity's offset and length in UTF-16 code units, so both sides are encoded
    // once and compared as slices; indexing by `char` misaligns the span and can panic on multi-byte
    // input.
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let wanted: Vec<u16> = username.encode_utf16().collect();
    entities.iter().any(|entity| {
        matches!(entity.kind, MessageEntityKind::Mention)
            && utf16.get(entity.offset..entity.offset.saturating_add(entity.length))
                == Some(wanted.as_slice())
    })
}

fn is_reply_to_bot(msg: &Message, bot_user_id: teloxide::types::UserId) -> bool {
    msg.reply_to_message()
        .and_then(|r| r.from.as_ref())
        .is_some_and(|u| u.id == bot_user_id)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::domain::vo::EntryAudience;

    const BOT_USERNAME: &str = "example_bot";
    const BOT_ID: u64 = 42;

    fn bot() -> BotIdentity<'static> {
        BotIdentity {
            username: BOT_USERNAME,
            user_id: teloxide::types::UserId(BOT_ID),
        }
    }

    /// A teloxide `Message` built from the JSON a real update carries; a media fixture without its
    /// `mime_type` decodes as the untagged kind's `Empty` arm, so every fixture spells it.
    fn arrival_of(content: Value) -> Arrival {
        arrival_of_in(content, "private").expect("an arrival")
    }

    /// The same message as an edit: the platform reports an edit through a second filter, and only
    /// a creation is read as an invocation.
    fn edited(content: Value) -> Arrival {
        let mut message = json!({
            "message_id": 1,
            "date": 0,
            "edit_date": 1,
            "chat": { "id": 1, "type": "private" },
        });
        let base = message.as_object_mut().expect("object literal");
        for (key, value) in content.as_object().expect("object literal") {
            base.insert(key.clone(), value.clone());
        }
        let message: Message = serde_json::from_value(message).expect("teloxide message fixture");
        arrival(
            &message,
            &bot(),
            ChangeKind::Edited {
                edited_at: jiff::Timestamp::from_second(1).expect("a valid second"),
            },
        )
        .expect("a readable message")
        .expect("an arrival")
    }

    fn arrival_of_in(content: Value, chat_type: &str) -> Option<Arrival> {
        let mut message = json!({
            "message_id": 1,
            "date": 0,
            "chat": { "id": 1, "type": chat_type },
        });
        let base = message.as_object_mut().expect("object literal");
        for (key, value) in content.as_object().expect("object literal") {
            base.insert(key.clone(), value.clone());
        }
        let message: Message = serde_json::from_value(message).expect("teloxide message fixture");
        arrival(&message, &bot(), ChangeKind::Created { edited_at: None })
            .expect("a readable message")
    }

    /// The caption the message carries, read off its attachment.
    fn caption_of(content: Value) -> Option<String> {
        arrival_of(content)
            .attachments
            .first()
            .and_then(|attachment| attachment.caption.clone())
    }

    #[test]
    fn a_media_message_carries_exactly_one_attachment() {
        for content in [
            json!({ "voice": { "file_id": "f", "file_unique_id": "u", "duration": 1, "mime_type": "audio/ogg" } }),
            json!({ "photo": [{ "file_id": "f", "file_unique_id": "u", "width": 1, "height": 1 }] }),
        ] {
            assert_eq!(arrival_of(content).attachments.len(), 1);
        }
    }

    #[test]
    fn a_document_keeps_its_kind_and_file_name() {
        let extracted = arrival_of(json!({
            "document": {
                "file_id": "f",
                "file_unique_id": "u",
                "file_name": "report.pdf",
                "mime_type": "application/pdf",
            },
        }));

        assert_eq!(extracted.attachments[0].kind, AttachmentKind::Document);
        assert_eq!(
            extracted.attachments[0].meta.file_name.as_deref(),
            Some("report.pdf")
        );
    }

    #[test]
    fn a_sticker_keeps_its_emoji_as_the_hint() {
        let extracted = arrival_of(json!({
            "sticker": {
                "file_id": "f",
                "file_unique_id": "u",
                "type": "regular",
                "width": 1,
                "height": 1,
                "is_animated": false,
                "is_video": false,
                "emoji": "🙂",
            },
        }));

        assert_eq!(extracted.attachments[0].kind, AttachmentKind::Sticker);
        assert_eq!(extracted.attachments[0].meta.hint.as_deref(), Some("🙂"));
    }

    #[test]
    fn voice_and_animation_keep_their_caption() {
        assert_eq!(
            caption_of(json!({
                "voice": {
                    "file_id": "f",
                    "file_unique_id": "u",
                    "duration": 1,
                    "mime_type": "audio/ogg",
                },
                "caption": "你好",
            })),
            Some("你好".to_owned())
        );
        assert_eq!(
            caption_of(json!({
                "animation": {
                    "file_id": "f",
                    "file_unique_id": "u",
                    "width": 1,
                    "height": 1,
                    "duration": 1,
                    "mime_type": "video/mp4",
                },
                "caption": "你好",
            })),
            Some("你好".to_owned())
        );
    }

    /// Telegram gives these kinds no caption; one in the payload is dropped rather than stored.
    #[test]
    fn kinds_telegram_gives_no_caption_stay_empty() {
        assert_eq!(
            caption_of(json!({
                "sticker": {
                    "file_id": "f",
                    "file_unique_id": "u",
                    "type": "regular",
                    "width": 1,
                    "height": 1,
                    "is_animated": false,
                    "is_video": false,
                },
                "caption": "ignored",
            })),
            None
        );
        assert_eq!(
            caption_of(json!({
                "video_note": {
                    "file_id": "f",
                    "file_unique_id": "u",
                    "length": 1,
                    "duration": 1,
                },
                "caption": "ignored",
            })),
            None
        );
    }

    /// The wire's own media type is what the reader gets, so a platform whose audio is not Ogg is
    /// read as what it is.
    #[test]
    fn the_wire_media_type_becomes_the_codec() {
        let voice = arrival_of(json!({
            "voice": { "file_id": "f", "file_unique_id": "u", "duration": 1, "mime_type": "audio/ogg" },
        }));
        assert_eq!(voice.attachments[0].meta.media_codec, Some(MediaCodec::Ogg));

        let audio = arrival_of(json!({
            "audio": { "file_id": "f", "file_unique_id": "u", "duration": 1, "mime_type": "audio/mp4" },
        }));
        assert_eq!(audio.attachments[0].meta.media_codec, Some(MediaCodec::Mp4));
    }

    /// A private chat is aimed at the bot; a group message is not unless it names or answers it.
    #[test]
    fn the_audience_follows_who_the_message_is_for() {
        assert_eq!(
            arrival_of(json!({ "text": "hi" })).audience,
            EntryAudience::Agent
        );

        assert_eq!(
            arrival_of_in(json!({ "text": "hi" }), "supergroup")
                .expect("an arrival")
                .audience,
            EntryAudience::Bystanders
        );

        let mentioned = arrival_of_in(
            json!({
                "text": "@example_bot hello",
                "entities": [{ "type": "mention", "offset": 0, "length": 12 }],
            }),
            "supergroup",
        )
        .expect("an arrival");
        assert_eq!(mentioned.audience, EntryAudience::Agent);

        let replied = arrival_of_in(
            json!({
                "text": "that again",
                "reply_to_message": {
                    "message_id": 3,
                    "date": 0,
                    "chat": { "id": 1, "type": "supergroup" },
                    "from": { "id": BOT_ID, "is_bot": true, "first_name": "bot" },
                },
            }),
            "supergroup",
        )
        .expect("an arrival");
        assert_eq!(replied.audience, EntryAudience::Agent);
    }

    /// Platform extras travel in the arrival's meta, never in its own fields: a themed chat and a
    /// quoted message are Telegram's own vocabulary, and the tail reads neither.
    #[test]
    fn platform_extras_travel_in_the_arrival_meta() {
        let extracted = arrival_of(json!({
            "text": "hi",
            "message_thread_id": 5,
            "quote": { "text": "an earlier line", "position": 0, "is_manual": true },
        }));

        let meta = extracted.meta.clone();
        assert_eq!(meta["platform"], "telegram", "{meta}");
        assert_eq!(meta["message_thread_id"], 5, "{meta}");
        assert_eq!(meta["quote"]["text"], "an earlier line", "{meta}");
        assert_eq!(extracted.text, "hi");
    }

    /// The bot's own START is the platform's convention, not a table row: answered, and its payload
    /// — a deep link's — ignored.
    #[test]
    fn a_start_is_the_platforms_own() {
        for text in ["/start", "/start ref-42", "/start@example_bot"] {
            assert!(
                matches!(
                    arrival_of(json!({ "text": text })).text_kind,
                    ArrivalText::Greeting
                ),
                "{text}"
            );
        }
        assert!(arrival_of_in(json!({ "text": "/start@other_bot" }), "supergroup").is_none());
    }

    #[test]
    fn a_command_is_read_into_its_name_and_payload() {
        let command = arrival_of(json!({ "text": "/status@example_bot now" }));
        match command.text_kind {
            ArrivalText::Command { name, payload } => {
                assert_eq!(name, "status");
                assert_eq!(payload, "now");
            }
            other => panic!("expected a command; got {}", kind_of(&other)),
        }

        assert!(matches!(
            arrival_of(json!({ "text": "plain" })).text_kind,
            ArrivalText::Ordinary
        ));
    }

    /// Editing text into a command does not run it: the edit is a change to a message, and the
    /// command it now spells was never a message of ours to run.
    #[test]
    fn an_edited_command_is_a_change_not_an_invocation() {
        assert!(matches!(
            edited(json!({ "text": "/digest 3" })).text_kind,
            ArrivalText::Ordinary
        ));
        assert!(matches!(
            edited(json!({ "text": "/start" })).text_kind,
            ArrivalText::Ordinary
        ));
        assert!(matches!(
            edited(json!({ "text": "plain" })).text_kind,
            ArrivalText::Ordinary
        ));
    }

    fn kind_of(kind: &ArrivalText) -> &'static str {
        match kind {
            ArrivalText::Ordinary => "ordinary",
            ArrivalText::Greeting => "greeting",
            ArrivalText::Command { .. } => "command",
        }
    }

    fn entity(kind: MessageEntityKind, offset: usize, length: usize) -> MessageEntity {
        MessageEntity {
            kind,
            offset,
            length,
        }
    }

    /// Two CJK characters and a space are three UTF-16 units, so `@bot` starts at 3 — a span counted
    /// in chars would point one unit short and match nothing.
    #[test]
    fn a_mention_span_is_measured_in_utf16_units() {
        assert!(check_entities(
            &[entity(MessageEntityKind::Mention, 3, 4)],
            "你好 @bot",
            "@bot"
        ));
        assert!(!check_entities(
            &[entity(MessageEntityKind::Mention, 3, 4)],
            "你好 @bob",
            "@bot"
        ));
    }

    #[test]
    fn only_a_mention_entity_counts() {
        assert!(!check_entities(
            &[entity(MessageEntityKind::Bold, 0, 4)],
            "@bot",
            "@bot"
        ));
    }

    #[test]
    fn a_span_past_the_end_is_not_a_panic() {
        assert!(!check_entities(
            &[entity(MessageEntityKind::Mention, 3, 99)],
            "你好 @bot",
            "@bot"
        ));
    }

    #[test]
    fn an_edit_carries_the_platform_time_and_a_creation_does_not_need_one() {
        let at = jiff::Timestamp::from_second(1_700_000_000).expect("a valid second");

        match change_kind(false, Some(at)) {
            Ok(ChangeKind::Created { edited_at }) => assert_eq!(edited_at, Some(at)),
            other => panic!("expected a creation; got {other:?}"),
        }
        match change_kind(false, None) {
            Ok(ChangeKind::Created { edited_at }) => assert_eq!(edited_at, None),
            other => panic!("expected a creation; got {other:?}"),
        }
        match change_kind(true, Some(at)) {
            Ok(ChangeKind::Edited { edited_at }) => assert_eq!(edited_at, at),
            other => panic!("expected an edit; got {other:?}"),
        }
        assert!(
            change_kind(true, None).is_err(),
            "an edit without the platform's own time is not a change to store"
        );
    }
}
