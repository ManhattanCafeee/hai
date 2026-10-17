//! Resolving an outgoing reference to the wire shape Telegram replies with: the target message's own
//! id and, when the sender quoted it, the excerpt. Which reference may be sent at all — that it
//! belongs to this chat and that the excerpt is verbatim — is `platform/outbound.rs`; this file only
//! has to make Telegram accept it, which it does not when the excerpt outgrows its field.

use teloxide::types::{MessageId as TgMessageId, ReplyParameters};

use crate::{
    domain::vo::{PlatformMessageMeta, TelegramMessageMeta, TelegramQuote},
    error::{AppError, Result},
};

pub(super) fn build_reply(external_id: &str, excerpt: Option<&str>) -> Result<ReplyParameters> {
    let id = external_id.parse::<i32>().map_err(|_| {
        AppError::message(format!(
            "platform message id {external_id} is not a Telegram id"
        ))
    })?;
    let mut params = ReplyParameters::new(TgMessageId(id));
    if let Some(excerpt) = excerpt {
        validate_quote(excerpt)?;
        params = params.quote(excerpt.to_owned());
    }
    Ok(params)
}

/// Telegram fails the whole send when the quoted part is longer than its field takes, so a quote that
/// cannot go out is an error here rather than a silently dropped field.
fn validate_quote(quote: &str) -> Result<()> {
    const QUOTE_MAX_CHARS: usize = 1024;

    if quote.chars().count() > QUOTE_MAX_CHARS {
        return Err(AppError::message(format!(
            "the excerpt is longer than Telegram takes ({QUOTE_MAX_CHARS} characters)"
        )));
    }
    Ok(())
}

/// The platform extras of an outgoing reply, recorded in the shape an incoming one uses.
pub(super) fn outgoing_quote_meta(quote: String) -> PlatformMessageMeta {
    PlatformMessageMeta::Telegram(TelegramMessageMeta {
        message_thread_id: None,
        quote: Some(TelegramQuote {
            text: quote,
            position: 0,
            is_manual: true,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_carries_its_target_id_and_excerpt() {
        let params = build_reply("7", Some("a part")).expect("a Telegram reply");
        assert_eq!(params.message_id, TgMessageId(7));
        assert_eq!(params.quote.as_deref(), Some("a part"));

        let params = build_reply("7", None).expect("a Telegram reply");
        assert!(params.quote.is_none());
    }

    #[test]
    fn a_target_that_is_not_a_telegram_id_is_refused() {
        let err = build_reply("not-an-id", None).unwrap_err();
        assert!(
            err.to_string().contains("not-an-id"),
            "the id belongs in the error: {err}"
        );
    }

    /// Telegram fails the whole send on a quote its field cannot hold, so the check has to hold here.
    #[test]
    fn an_excerpt_past_the_field_ceiling_is_refused() {
        let long = "x".repeat(1025);
        let err = build_reply("7", Some(&long)).unwrap_err().to_string();
        assert!(
            err.contains("longer than Telegram takes"),
            "the ceiling says what it is: {err}"
        );

        let at_limit = "y".repeat(1024);
        assert!(build_reply("7", Some(&at_limit)).is_ok());
    }
}
