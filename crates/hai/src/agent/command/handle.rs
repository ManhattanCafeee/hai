use super::{Command, CommandContext, Invocation, LanguageArg, Reply, Said, SystemCommand};
use crate::{
    agent::copy::HostCopy,
    agentcore::Language,
    domain::vo::{AgentCommand, ChatEntry, EntryAudience, EntryKind},
    error::Result,
};

pub(crate) async fn execute(ctx: &CommandContext, invocation: Invocation) -> Result<Reply> {
    match invocation {
        Invocation::Malformed => said(ctx, HostCopy::Unread).await,
        Invocation::Ready(Command::System(command)) => answer(ctx, command).await,
        Invocation::Ready(Command::Agent(command)) => {
            enqueue(ctx, command).await?;
            Ok(Reply::Silent)
        }
    }
}

/// Answers a command the host handles itself.
async fn answer(ctx: &CommandContext, command: SystemCommand) -> Result<Reply> {
    match command {
        SystemCommand::Status => status(ctx).await,
        SystemCommand::Lang(arg) => language(ctx, arg).await,
    }
}

async fn said(ctx: &CommandContext, copy: HostCopy) -> Result<Reply> {
    let language = host_language(ctx).await?;
    Ok(Reply::Said(Said { copy, language }))
}

async fn host_language(ctx: &CommandContext) -> Result<Language> {
    ctx.settings.language(ctx.chat_id).await
}

async fn status(ctx: &CommandContext) -> Result<Reply> {
    let language = host_language(ctx).await?;
    let body = match ctx.agents.get(ctx.chat_id).await {
        Some(agent) => agent.status_text(language).await,
        None => None,
    };
    Ok(Reply::Said(Said {
        copy: HostCopy::Status(body),
        language,
    }))
}

/// The preference is written before the reply is spoken, so a set language is confirmed in the
/// language it just became.
async fn language(ctx: &CommandContext, arg: LanguageArg) -> Result<Reply> {
    let copy = match arg {
        LanguageArg::Current => HostCopy::LanguageCurrent,
        LanguageArg::Reset => {
            ctx.settings.reset_language(ctx.chat_id).await?;
            HostCopy::LanguageReset
        }
        LanguageArg::Set(language) => {
            ctx.settings.set_language(ctx.chat_id, language).await?;
            HostCopy::LanguageSet
        }
        LanguageArg::Unsupported => HostCopy::LanguageUnsupported,
    };
    said(ctx, copy).await
}

/// Hands the agent an instruction and stays quiet — the answer arrives in the conversation.
async fn enqueue(ctx: &CommandContext, command: AgentCommand) -> Result<()> {
    let entry = ChatEntry::new(
        EntryAudience::Agent,
        EntryKind::Command(command),
        jiff::Timestamp::now(),
    );
    ctx.services
        .conversation
        .append_entry(ctx.chat_id, &entry)
        .await?;
    ctx.agents.get_or_create(ctx.chat_id).await.notify();
    Ok(())
}
