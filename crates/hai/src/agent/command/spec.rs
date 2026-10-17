//! The command table: identity, menu copy and argument grammar for every command.

use strum::{EnumIter, IntoEnumIterator};

use crate::{
    agent::command::{Command, Invocation, LanguageArg, SystemCommand},
    agentcore::{Language, PerCatalog, TextCatalog},
    domain::vo::AgentCommand,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter)]
pub(crate) enum CommandName {
    Status,
    Lang,
    OrganizeMemory,
    Explain,
    Digest,
}

impl CommandName {
    /// The wire name, without the platform's prefix.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Lang => "lang",
            Self::OrganizeMemory => "organizememory",
            Self::Explain => "explain",
            Self::Digest => "digest",
        }
    }

    /// Other spellings the same command answers to.
    pub(crate) fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Explain => &["explan"],
            _ => &[],
        }
    }

    /// The menu copy for a client reading `catalog`.
    pub(crate) fn description(self, catalog: TextCatalog) -> &'static str {
        self.copy().get(catalog)
    }

    fn copy(self) -> PerCatalog<&'static str> {
        match self {
            Self::Status => STATUS_COPY,
            Self::Lang => LANG_COPY,
            Self::OrganizeMemory => ORGANIZE_MEMORY_COPY,
            Self::Explain => EXPLAIN_COPY,
            Self::Digest => DIGEST_COPY,
        }
    }

    /// The invocation a payload denotes.
    pub(crate) fn parse(self, payload: &str) -> Invocation {
        let payload = payload.trim();
        match self {
            Self::Status => unit(payload, Command::System(SystemCommand::Status)),
            Self::OrganizeMemory => unit(payload, Command::Agent(AgentCommand::OrganizeMemory)),
            Self::Lang => Invocation::Ready(Command::System(SystemCommand::Lang(match payload {
                "" => LanguageArg::Current,
                "reset" => LanguageArg::Reset,
                code => match code.parse::<Language>() {
                    Ok(language) => LanguageArg::Set(language),
                    Err(_) => LanguageArg::Unsupported,
                },
            }))),
            Self::Explain => {
                Invocation::Ready(Command::Agent(AgentCommand::Explain(match payload {
                    "" => None,
                    target => Some(target.to_string()),
                })))
            }
            Self::Digest if payload.is_empty() => {
                Invocation::Ready(Command::Agent(AgentCommand::Digest(DIGEST_DEFAULT_DAYS)))
            }
            Self::Digest => match payload.parse::<u32>() {
                Ok(days) => Invocation::Ready(Command::Agent(AgentCommand::Digest(
                    days.max(MIN_DIGEST_DAYS),
                ))),
                Err(_) => Invocation::Malformed,
            },
        }
    }
}

/// A `/digest` day count covers at least one day: digesting none of them is not a request.
const MIN_DIGEST_DAYS: u32 = 1;

/// The day count `/digest` runs over when it is given none.
const DIGEST_DEFAULT_DAYS: u32 = 7;

/// A command that takes no argument at all.
fn unit(payload: &str, command: Command) -> Invocation {
    if payload.is_empty() {
        Invocation::Ready(command)
    } else {
        Invocation::Malformed
    }
}

const STATUS_COPY: PerCatalog<&'static str> = PerCatalog {
    zh: "查看运行状态",
    en: "Show the agent status",
};

const LANG_COPY: PerCatalog<&'static str> = PerCatalog {
    zh: "设置聊天语言",
    en: "Set the language for this chat",
};

const ORGANIZE_MEMORY_COPY: PerCatalog<&'static str> = PerCatalog {
    zh: "整理记忆与话题",
    en: "Organize memory and topics",
};

const EXPLAIN_COPY: PerCatalog<&'static str> = PerCatalog {
    zh: "解释指定内容或当前上下文",
    en: "Explain a given target or the current context",
};

const DIGEST_COPY: PerCatalog<&'static str> = PerCatalog {
    zh: "总结近期值得一提的对话",
    en: "Summarize noteworthy recent conversation",
};

/// Resolves a wire name to its command. Names are exact: the command contract does not inherit the
/// platform's spelling habits.
pub(crate) fn find(name: &str) -> Option<CommandName> {
    CommandName::iter()
        .find(|candidate| candidate.name() == name || candidate.aliases().contains(&name))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn names_are_unique_and_aliases_do_not_collide() {
        let names: HashSet<&str> = CommandName::iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), CommandName::iter().count());

        let mut spellings = names.clone();
        for command in CommandName::iter() {
            for alias in command.aliases() {
                assert!(spellings.insert(alias), "{alias} collides");
            }
        }
    }

    #[test]
    fn find_resolves_names_and_aliases_exactly() {
        assert_eq!(find("lang"), Some(CommandName::Lang));
        assert_eq!(find("explan"), Some(CommandName::Explain));
        assert_eq!(find("Lang"), None);
        assert_eq!(find("unknown"), None);
        assert_eq!(find(""), None);
    }

    #[test]
    fn parse_reads_each_payload() {
        use Command::{Agent, System};
        use LanguageArg::{Current, Reset, Set, Unsupported};
        use SystemCommand::Lang;

        let ready = Invocation::Ready;
        let malformed = Invocation::Malformed;

        assert_eq!(CommandName::Status.parse("x"), malformed);
        assert_eq!(CommandName::OrganizeMemory.parse("x"), malformed);
        assert_eq!(
            CommandName::OrganizeMemory.parse(""),
            ready(Agent(AgentCommand::OrganizeMemory))
        );
        assert_eq!(CommandName::Lang.parse("  "), ready(System(Lang(Current))));
        assert_eq!(CommandName::Lang.parse("reset"), ready(System(Lang(Reset))));
        assert_eq!(
            CommandName::Lang.parse("JA"),
            ready(System(Lang(Set(Language::Ja))))
        );
        assert_eq!(
            CommandName::Lang.parse("EN-GB"),
            ready(System(Lang(Set(Language::EnGb))))
        );
        assert_eq!(
            CommandName::Lang.parse("jp"),
            ready(System(Lang(Unsupported)))
        );
        assert_eq!(
            CommandName::Explain.parse(""),
            ready(Agent(AgentCommand::Explain(None)))
        );
        assert_eq!(
            CommandName::Explain.parse("a  b"),
            ready(Agent(AgentCommand::Explain(Some("a  b".into()))))
        );
        assert_eq!(
            CommandName::Digest.parse(""),
            ready(Agent(AgentCommand::Digest(DIGEST_DEFAULT_DAYS)))
        );
        assert_eq!(
            CommandName::Digest.parse("003"),
            ready(Agent(AgentCommand::Digest(3)))
        );
        assert_eq!(
            CommandName::Digest.parse("0"),
            ready(Agent(AgentCommand::Digest(1)))
        );
        assert_eq!(CommandName::Digest.parse("abc"), malformed);
        assert_eq!(CommandName::Digest.parse("3 extra"), malformed);
    }
}
