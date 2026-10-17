//! The menu scopes to push: the catalog a client with no scope of its own reads, then one per client
//! language code reading another, because Telegram hands a client the most specific scope it matches.

use strum::IntoEnumIterator;
use teloxide::{
    Bot,
    payloads::SetMyCommandsSetters,
    prelude::Requester,
    types::{BotCommand, BotCommandScope},
};

use crate::{
    agent::command::CommandName,
    agentcore::{Language, PerCatalog, TextCatalog},
};

/// The catalog a client whose language has no scope of its own reads: every language but Chinese
/// reads English, so English is the fallback and only another catalog needs a code.
const FALLBACK: TextCatalog = TextCatalog::En;

/// Each scope as the client language code Telegram matches by and the catalog it reads; `None` is the
/// fallback. Telegram takes a two-letter ISO 639-1 `language_code`, so a code is pushed once — a
/// regional variant shares its base language's entry.
pub(super) fn scopes() -> Vec<(Option<&'static str>, TextCatalog)> {
    let mut scopes = vec![(None, FALLBACK)];
    for language in Language::iter() {
        let (code, catalog) = (language.client_code(), language.text_catalog());
        if catalog != FALLBACK && !scopes.iter().any(|(pushed, _)| *pushed == Some(code)) {
            scopes.push((Some(code), catalog));
        }
    }
    scopes
}

/// The platform's own convention, listed first: Telegram shows a START button for every bot, and the
/// menu says what it does in the client's language.
const START_COPY: PerCatalog<&'static str> = PerCatalog {
    zh: "开始使用",
    en: "Start the bot",
};

/// The menu Telegram shows a client, in the order the command table declares. The name goes without
/// the slash — Telegram's `BotCommand.command` allows only lowercase letters, digits and
/// underscores — and the client renders the slash itself.
pub(super) fn bot_commands(catalog: TextCatalog) -> Vec<BotCommand> {
    let mut commands = vec![BotCommand::new("start", START_COPY.get(catalog))];
    commands.extend(
        CommandName::iter()
            .map(|command| BotCommand::new(command.name(), command.description(catalog))),
    );
    commands
}

/// Pushes every menu scope. A failure only warns: the bot still answers commands, it just shows an
/// older menu.
pub(super) async fn install(bot: &Bot) {
    for (language_code, catalog) in scopes() {
        let request = bot
            .set_my_commands(bot_commands(catalog))
            .scope(BotCommandScope::Default);
        let result = match language_code {
            Some(code) => request.language_code(code).await,
            None => request.await,
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, language_code = ?language_code, "Failed to set bot commands");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_cover_the_fallback_and_every_code_reading_another_catalog() {
        assert_eq!(
            scopes(),
            vec![(None, TextCatalog::En), (Some("zh"), TextCatalog::Zh)]
        );
    }

    #[test]
    fn menu_lists_every_command_by_its_bare_name() {
        for catalog in TextCatalog::iter() {
            let listed: Vec<String> = bot_commands(catalog)
                .into_iter()
                .map(|command| command.command)
                .collect();

            assert_eq!(
                listed.first().map(String::as_str),
                Some("start"),
                "the platform convention comes first"
            );
            for command in CommandName::iter() {
                assert!(
                    listed.contains(&command.name().to_owned()),
                    "{} is missing from the menu",
                    command.name()
                );
            }
            assert_eq!(
                listed.len(),
                CommandName::iter().count() + 1,
                "the menu lists the table and nothing else"
            );
        }
    }
}
