//! Command semantics: what each command is, what a payload denotes, and what the host says back.

mod context;
mod handle;
mod spec;

pub(crate) use context::CommandContext;
pub(crate) use handle::execute;
pub(crate) use spec::{CommandName, find};

use crate::{agent::copy::HostCopy, agentcore::Language, domain::vo::AgentCommand};

/// A payload the table has judged: either a command it could read, or one it could not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Invocation {
    Ready(Command),
    Malformed,
}

/// A command the host answers itself, without the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SystemCommand {
    Status,
    Lang(LanguageArg),
}

/// A command the host answers itself, or an instruction it hands the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    System(SystemCommand),
    Agent(AgentCommand),
}

/// What `/lang` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LanguageArg {
    Current,
    Reset,
    Set(Language),
    Unsupported,
}

/// What the host replies with. An agent task is answered by the agent inside the conversation, so
/// the host itself says nothing — `Silent` is that silence, not a missing case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    Silent,
    Said(Said),
}

/// One reply the host speaks, in the language it has to be spoken in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Said {
    pub(crate) copy: HostCopy,
    pub(crate) language: Language,
}
