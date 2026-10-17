# Commands

> The command table, who a slash message belongs to, and the menu. Must-read when adding a command or changing what one replies.

## Overview

A command is one row of `agent/command/spec.rs:CommandName` — its name, its aliases, its menu copy and the grammar of its payload. Nothing else holds that list: the menu, the argument parsing and the answering all read the row. Telegram owns the adapter: deciding whose message a slash line is, rendering a menu, sending a reply (`platform/telegram/`).

## Design

### The declaration

Each identity answers three questions:

- `name` and `aliases` — the wire spelling. Names match **exactly**: the command contract does not inherit a platform's spelling habits, so `/explan` is `Explain` and `/Lang` is an ordinary message.
- `description(TextCatalog)` — the menu copy, in the text catalog the client's language reads.
- `parse(payload) -> Invocation` — the invocation the payload denotes, and the only place a `Command` is built. `Invocation::Ready` carries the command itself, typed: an `AgentCommand` for the agent class, a `SystemCommand` for the host's own (`LanguageArg` lives there); `Invocation::Malformed` means the payload does not fit the command at all.

`find` resolves a name or an alias to a row. `/digest`'s own grammar floors its count and defaults an absent one (`spec.rs`), so what reaches the agent is a count of at least one day.

### Who a message belongs to

`platform/telegram/command.rs:classify` answers **ownership**, independently of recognition:

| `CommandOwner` | Meaning |
|---|---|
| `NotACommand` | not a slash message at all |
| `Theirs` | `/name@other_bot` — a command for another bot |
| `Ours { name, payload }` | our slash message, known name or not |

`/start` never reaches the table: it is the platform's own convention, answered before the table is consulted.

Ownership is judged on the `@` suffix, matched against the bot's own name as written — the same exactness a mention gets (`platform/telegram/arrival.rs:is_mentioning_user`) — and the payload is sliced at the first whitespace so the user's own spacing survives. `/foo@` lands in `Theirs`; `/foo` and `/Lang` land in `Ours` with the name's case intact, leaving `find` to reject them.

Recognition then splits `Ours` in two: a name the table knows goes to `agent/command/handle.rs:execute` with its `Invocation`; a name it does not know is an ordinary message, because a person typing an unknown slash is still talking to the agent.

### What enters the conversation

A message is stored only when it can form a complete exchange:

| Message | `message` table | Answer |
|---|---|---|
| our command, payload readable | no | host copy, or the agent inside the conversation |
| our command, payload unreadable | no | host copy (`HostCopy::Unread`), no wake |
| `/name@other_bot` | no | dropped, with a debug log |
| anything else | yes | the agent, woken by the usual reason |

A command for another bot is never stored because its other half never arrives: Telegram does not deliver messages from bots to bots, so the reply is unreachable by construction. A recognized command writes no message: an agent task joins the accumulation as a command entry carrying the `AgentCommand` itself, and a host operation leaves no trace because the host answered it on the spot.

### Answering

`execute` takes a `CommandContext` — the chat, the `Settings` facade, and the `AgentRegistry` — assembled by the shared inbound tail (`agent/intake.rs:Intake::run_command`), and answers with a `Reply`:

- `Reply::Said` carries a `HostCopy` and the language it must be spoken in; the host renders it through `agent/copy.rs:HostCopy::render`, and the platform delivers what it is handed.
- `Reply::Silent` is an agent task: it joins the chat's accumulation as a command entry, and the answer arrives in the conversation.

The language is resolved once per reply through `agent/settings.rs:Settings::language` — the same resolution the agent itself uses — and for `/lang` it is resolved *after* the preference is written, so setting a language is confirmed in the language it just became. `/status` renders the agent's own block (`agent/runtime/chat/handle.rs:ChatAgentStatus`) in that one language.

### Arguments

- `/status` — the agent's status block; when the agent is not running the host copy says so.
- `/lang [code|reset]` — bare reports the current language, `reset` deletes the per-chat row so the configured default applies again, and any other value is parsed case-insensitively as `Language`, so `/lang JA` works; an unknown code is answered as such rather than as a generic unreadable payload. Any whitelisted member may set it, and it is per chat, never per user.
- `/organizememory` — enqueue an `organize_memory` entry.
- `/explain [target]`, alias `/explan` — enqueue an `explain` entry carrying the target (empty when none was named); without a target the agent explains the object most recently mentioned and not yet explained.
- `/digest [N]` — enqueue a `digest` entry carrying the day count, N defaulting to 7 and floored at 1.

The command itself is `domain/vo/command.rs:AgentCommand` — a domain value, because a `Command` entry stores it; the instruction the agent reads for one lives in `agent/runtime/event/wake.rs:AgentCommand::instruction`, in the chat's language.

### Host copy and the menu

Host copy lives in `agent/copy.rs`: `HostCopy` says what the host says — the language replies are their own variants — and `HostCopy::render` says it in the chat's built-in set. The platform only delivers what it is handed — MarkdownV2 and the plain-text fallback live in `platform/telegram/service.rs:send_markdown_with_fallback`, behind `agent/channel.rs:ChannelAccess::notice`, which the inbound tail's host-copy path calls. `HostCopy::Unauthorized` is produced by the whitelist path and never by a command, and `HostCopy::Greeting` by `/start`, which is the platform's own convention rather than a table row: `platform/telegram/arrival.rs` reads it off a message that names it, before the table is consulted (a deep link's payload is ignored), and `platform/telegram/menu.rs:bot_commands` lists it first with copy of its own.

The menu is generated from the declaration table in declaration order (`platform/telegram/menu.rs:bot_commands`). Telegram takes a bare name — its `BotCommand.command` allows only lowercase letters, digits and underscores — and renders the slash itself. Scopes are derived, not listed: `platform/telegram/menu.rs:scopes` pushes the English catalog as the scope without a `language_code` — the fallback a client whose language has no scope of its own reads — plus one `language_code` scope per client code reading another catalog, because Telegram hands a client the most specific scope it matches; the code is a two-letter ISO 639-1 one, the only form Telegram accepts.

## Boundaries

- No platform type reaches `agent/command/`: the wire form and the rendering stay at the platform edge, and the semantics take a `CommandContext`.
- Whitelisting precedes all of it (`agent/intake.rs:Intake::accept` asks `agent/channel.rs:ChannelAccess::admits`); a chat outside the allowlist never reaches the command table, and its `/start` is the one thing the platform answers there.
- A command is not a tool: a host operation never wakes the agent, and an agent task owes the host no copy.

## Trade-offs and Reasons

- **Ownership is judged before recognition** — the `@` suffix is the user's own statement of who the message is for, so it decides even for names the table knows; Telegram's own delivery rules agree (`docs/topics/platform.md`).
- **A recognized command writes no message** — its accumulation entry is the single representation; storing the call as a message as well would put the same act in the conversation twice.
- **An unreadable payload is answered, not handed on** — the user addressed our command, so the host says it could not read it; passing it to the agent would produce a second answer to the same message.
- **Exact names** — matching the table literally keeps the contract independent of the platform; case-insensitivity would have to be re-decided per platform.

## Pitfalls / common mistakes

- **A new command needs its whole row**: identity, both descriptions, the `parse` arm and the `execute` arm. The matches are exhaustive, so a missing arm is a compile error.
- **`@bot` matching is exact** — `classify` compares the suffix with the bot's own name as written and lowercases nothing else; only `/lang` normalizes its own argument.
- **`HostCopy::Status(None)` is the host's own copy**: `/status` answers even when no agent exists for the chat.
- **A menu name carries no slash** — Telegram rejects or mis-renders one, and the client adds it.
- **A payload sliced from `split_whitespace` would lose the user's spacing** — `classify` slices it out of the original text instead.

## Evolution Direction

- A second platform takes the declaration table as it stands and owns only the classification and the menu, the way `platform/telegram/command.rs` does: the orchestration above them is `agent/intake.rs:Intake`, shared by every platform (`docs/architecture.md`).
