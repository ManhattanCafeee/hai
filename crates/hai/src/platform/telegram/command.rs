//! The Telegram side of commands: whose command a slash message is.

/// Whose message this is. A name addressed to another bot is a conversation whose other half we can
/// never see: Telegram does not deliver messages from bots to bots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommandOwner<'a> {
    Ours { name: &'a str, payload: &'a str },
    Theirs,
    NotACommand,
}

/// Splits a message into the command name and its payload, and decides whether the command is ours.
/// Both spellings are compared as they are — the `@` suffix against the bot's own name, the name
/// against the command table — and the payload keeps its own spacing, being the user's words
/// rather than a token list.
pub(super) fn classify<'a>(text: &'a str, bot_username: &str) -> CommandOwner<'a> {
    let Some(rest) = text.strip_prefix('/') else {
        return CommandOwner::NotACommand;
    };
    let head = rest.split_whitespace().next().unwrap_or_default();
    let (name, target) = match head.split_once('@') {
        Some((name, target)) => (name, Some(target)),
        None => (head, None),
    };
    if let Some(target) = target
        && target != bot_username
    {
        return CommandOwner::Theirs;
    }
    let payload = match text.find(char::is_whitespace) {
        Some(at) => text[at..].trim_start(),
        None => "",
    };
    CommandOwner::Ours { name, payload }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::command::find;

    const BOT: &str = "example_bot";

    #[test]
    fn our_commands_keep_their_name_and_payload() {
        let ours = |name, payload| CommandOwner::Ours { name, payload };

        assert_eq!(classify("/status", BOT), ours("status", ""));
        assert_eq!(classify("/status@example_bot", BOT), ours("status", ""));
        assert_eq!(
            classify("/explain a  b   c", BOT),
            ours("explain", "a  b   c")
        );
        assert_eq!(classify("/explain   ", BOT), ours("explain", ""));
        assert_eq!(classify("/lang\ten", BOT), ours("lang", "en"));
    }

    /// The bot's own name is matched as written, the way a mention is.
    #[test]
    fn the_bot_name_is_matched_as_written() {
        assert_eq!(classify("/status@Example_Bot", BOT), CommandOwner::Theirs);
    }

    /// A slash message we own but do not know stays ours; `find` is what rejects the name.
    #[test]
    fn an_unknown_name_is_still_ours() {
        assert_eq!(
            classify("/foo", BOT),
            CommandOwner::Ours {
                name: "foo",
                payload: ""
            }
        );
        assert_eq!(
            classify("/Lang", BOT),
            CommandOwner::Ours {
                name: "Lang",
                payload: ""
            }
        );
        assert_eq!(
            classify("/", BOT),
            CommandOwner::Ours {
                name: "",
                payload: ""
            }
        );
        assert_eq!(find("foo"), None);
        assert_eq!(find("Lang"), None);
    }

    #[test]
    fn another_bots_command_and_plain_text_are_not_ours() {
        assert_eq!(classify("/status@other_bot", BOT), CommandOwner::Theirs);
        assert_eq!(classify("/foo@", BOT), CommandOwner::Theirs);
        assert_eq!(classify("plain text", BOT), CommandOwner::NotACommand);
        assert_eq!(classify("  /foo", BOT), CommandOwner::NotACommand);
    }
}
