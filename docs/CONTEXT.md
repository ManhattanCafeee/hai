# HAI domain language

HAI is a long-term companion AI assistant running as a Telegram bot. This document is the authority on the project's language: what each word *is*. Mechanisms live in `docs/architecture.md` and `docs/topics/*.md`.

## Language

### Entity domain

**Chat**: a conversation on a platform (Telegram private, group or channel) — the boundary a ChatAgent runs in.
_Avoid_: "room", "conversation"

**chat language**: the language a Chat is spoken in — set per chat by `/lang`, otherwise the configured default. Not the user's own language, and not a fence: the agent still translates, practises a language or quotes someone when asked.
_Avoid_: "locale" (that is a platform or client concept), "user language"

**text catalog**: the text a chat language reads — Chinese has a catalog of its own and every other language reads the English one; it spans the built-in persona and chat-type layers, the wake instruction and situation text, host copy and status labels, while the `agents` and `wrap-up` layers carry one text every catalog reads, and an operator override replaces a single layer, never a catalog.
_Avoid_: "message catalog" (a Message is a chat message), "resource bundle", "locale bundle", "built-in text set"

**client code**: the two-letter ISO 639-1 code a client platform matches a client's language by (`agentcore/language/mod.rs:Language::client_code`); a regional variant has no code of its own, so `en-gb` is served by `en`. The chat language's stored value is not this — that tag is `Language::as_str`.
_Avoid_: "locale" (the client's own setting, which the bot never reads), "language code" (ambiguous with the stored tag)

**Account**: one person's identity on one platform, carrying that platform's metadata.
_Avoid_: "user" (the user is the real person)

**Identity**: a cross-platform person binding several Accounts; reserved — the table, repo and `domain/service/identity.rs:IdentityService` are in place, and no live path creates or binds one yet.

**Topic**: one thing tracked over a span of time — a title, a summary, append-only notes, and the messages it covers. Independent of chapter boundaries.
_Avoid_: "theme", "thread" (a thread is a platform concept)

**note**: one append-only piece of raw material inside a Topic; the summary is a projection over the notes, so rewriting it loses nothing.
_Avoid_: "detail", "log"

**need-close**: the hint rendered on a Topic that has been idle past `topic_idle_hours`, telling the model the topic can be closed. Rendering only — nothing is stored, and closing does not freeze the record.
_Avoid_: "closed", "archive marker"

**recall**: the model's own lookup into past memory and topics — a semantic query, lexical keywords and a time range, fused into one ranking.
_Avoid_: bare "RAG", bare "search"

**Memory**: one persisted atomic fact (a user fact, a note or a knowledge entry) with a lifecycle and a pinned flag that puts it in the chapter-opening block.
_Avoid_: "knowledge base"

**Perception**: the cached result of analyzing one attachment or URL — a base transcription, plus a targeted judgment when the caller names a focus. Its key is `subject_kind` (`handle` = `<platform>:<file_unique_id>`, `url`) plus `subject`, parser and focus.

**attachment**: one occurrence of a file in one message — the occurrence id the model reads, the platform handle, the kind, the caption and the meta. Two occurrences of the same file are distinct rows that share one **Perception**.

**subject** / **subject_kind**: what a **Perception** row is cached against — `handle` spells `<platform>:<file_unique_id>`, falling back to the platform's download handle when no stable id was given, `url` a normalized URL, and the kind travels with the value (`domain/vo/subject.rs:Subject`). Every writer and reader spells it identically.
_Avoid_: "key" bare when the perception key is meant

**KnowledgeBase**: the global, curated document corpus, built from the CLI and read only by the agent. Distinct from Memory, which is chat-scoped personal memory; both exist.
_Avoid_: conflating the two

**KnowledgeDocument** / **KnowledgeChunk** / **Collection**: a document is the lifecycle unit (import, update, delete), a chunk is the retrieval unit, and a collection is the label a document carries — or none, for a document filed under nothing (`Collection::Uncategorised`).

**WebSearch**: live-web retrieval through search providers — candidate links with excerpts, neither curated nor verified. Distinct from knowledge-base retrieval (curated) and from `recall` (chat-scoped memory).

**web provider**: a supplier of the networking capability (parallel, exa, tavily); its identity, endpoint, tool names, credential requirement and capabilities are declared once, and both the search chain and `read`'s fetch chain use the same instances.
_Avoid_: bare "provider" when the model provider (`[providers.*]`) is meant

**provider availability**: whether a `[providers.*]` entry can be used — a credential (the kind's environment variable, or the block's `api-key`), or, for a keyless service, its endpoint answering; so a block declares an override (transport, endpoint, headers), not the provider's existence.
_Avoid_: calling `[providers.*]` "configuration" when availability is meant

**Message**: one message in a Chat — its own text plus its attachments, each an `attachment` row holding a platform file handle.
_Avoid_: "message record"

**Event**: one observable event of an agent run, its payload a tagged enum, persisted for auditing.
_Avoid_: "log" (log is tracing output)

**ConversationRecord**: the persisted conversation state — accumulated context messages, the chapter scalars, and the chat's `pending` accumulation. Its row is the one place a chat's state lives; the execution environment is rebuilt from it.
_Avoid_: conflating with the in-memory Conversation

**Chapter**: the segmentation unit of conversation history — the current chapter plus the retention summary of the previous one. Older content is discarded.

**Chapter reopen**: opening a new chapter once no run has happened for the idle timeout, or once the chapter's token count reaches `compact-token-threshold`. The wrap-up pass is one part of it, not the behavior itself.
_Avoid_: "compaction", "cleanup"

### Runtime domain

**ChatAgent**: the agent entity of one Chat — the counterpart the user talks to. It runs the event loop with an Idle/Busy state machine, is created lazily per Chat, and is rebuilt from the snapshot when needed.
_Avoid_: "session", "instance"

**Sandbox**: the environment the agent's tools act on — where a command runs and which paths a tool may open (`agent/runtime/sandbox/mod.rs:Sandbox`; two variants, picked from `[sandbox] enabled`: a container, or the host). The workspace and its read-only mounts are the tree both sides share, so `read` and `bash` reach the same files.
_Avoid_: "shell" for the environment as a whole

**Workspace**: the one writable directory of a Chat, given by `agent/runtime/workspace.rs:Workspace::for_chat` as `<data_dir>/workspaces/<chat id>`, bound at `/workspace` inside a container and the default working directory there; `workspace://<path>` is how a tool addresses it, so the directory's real location never enters the model's own spelling. A generated image lands in `generated/` inside it, and nothing reaches the chat except through `send_message`.

**Turn**: the execution unit in which the agent fully responds to one wake-up — the whole loop from taking the accumulated changes to replying, possibly several Steps and tool calls. It ends as Success, Steered (a new event interrupted it, and what it already did takes effect) or Failed (an exception, with no state side effects). Numbered per chapter.
_Avoid_: "round"

**Step**: one LLM call together with that call's tool results, numbered within its Turn. Recorded at runtime, never persisted.
_Avoid_: "step" for a process description rather than an LLM call

**AgentRole**: the role the model plays in one loop — `Default`, the companion in everyday conversation, or `WrapUp`, the chapter-close record-keeper whose text output is machine-extracted. A role owns its loop: prompt, options, tool surface, stopping and steering.
_Avoid_: conflating with ModelRole; bare "role"; "persona" (a future dynamic mask above the Default role's soul)

**ModelRole**: a named use of a model — vision, transcription, speech, embedding, image or rerank — selected through the `[model-roles]` config blocks rather than an enum. The main model is the default.
_Avoid_: "model config"

**ModelKind**: what a ModelRole asks a provider for — `chat`, `vision`, `transcription`, `embedding`, `image`, `speech`, `rerank`. A model kind is one endpoint class of a provider, not one model, and the role names it.
_Avoid_: calling it a "surface"; "capability" as a bare synonym; bare "kind" (the provider kind is the vendor preset, the model kind is the endpoint class)

**Scorer**: the retrieval scoring stage as the domain consumes it (`agentcore/api/rerank.rs:Scorer`): one rerank transport plus the budgets every call shares. Configured through `[model-roles.rerank]`; absent means retrieval keeps the fusion order.
_Avoid_: "judge" for this stage (a judgement model is a criteria-based verifier, which HAI does not run); "reranker" for the type (that is the model)

**soul**: the identity and voice layer — how the agent speaks, as one piece of free-form prose.

**agents**: the operational prompt layer — interaction rhythm plus memory and topic maintenance. Not the identity or voice, and not this repository's own AGENTS.md.

**context messages**: the accumulated sequence handed to the LLM — rendered messages, the chapter-opening block, and tool call/result chains, appended per Turn and persisted in the snapshot. Message is the stored content; context messages are the accumulated LLM product.

**ContextMeta**: the chapter's persisted scalars — context tokens, successful Turn count, Step count — reset on reopen.

**chat preference**: a per-Chat setting that overrides the configured default (currently the language the chat speaks); no row means the chat inherits the default, and the stored blob tolerates fields a newer build added. `agent/settings.rs:Settings` is where a preference and the configured default are resolved into the value a chat runs with.

**steering**: ending the current Turn early when a new event arrives, then opening a new Turn to handle it. Attention continuing, not work being preempted.

**attention window**: the spell during which the agent keeps attending — opened by a change addressed to it, or by its own reply — and inside which everything pending dispatches (`agent/runtime/chat/scheduler.rs:EventScheduler::on_addressed`, `:on_spoke`).
_Avoid_: bare "window" (that one is the context window), "attention span"

**Chapter wrap-up**: the pass that condenses a chapter into the next chapter's opening. It does not own topic lifecycle, runs with the same agent and tool surface as a normal turn, and its text output is machine-extracted.
_Avoid_: "summarize", "digest" (a summary is a conversational reply)

**ChatEntry**: one change to a chat as the state flow accumulates it — a message created, a message edited, a command, a scheduled task — carrying what happened and when, never the content (the message row holds that). Entries live in `conversation.pending`, ordered by when the change happened.
_Avoid_: "event" (the `event` table is the agent's audit trail)

**accumulation** (`pending`): the changes of a chat that no Turn has consumed yet. A Turn takes the whole accumulation and consumes it only on a normal end; a failed Turn leaves it for the next attempt, and a generation start re-feeds it.

**WakeEvent**: what a Turn reads its changes as — one reason per accumulated change, coalesced into the situation text the model sees. A `Command` reason renders into the instruction the model reads.

**WakeReason**: how one accumulated change reads: Observe, Direct, Mention, Scheduled or Command. Who a change was aimed at decides it, and a private chat always reads as Direct.

**command table**: the declaration of every command — name, aliases, menu copy and payload grammar — in `agent/command/spec.rs`.
_Avoid_: "command list", "registry" (the agent registry is a different thing)

**host operation**: a command the host answers on the spot, which never enters the conversation (`/start`, `/status`, `/lang`).
_Avoid_: "built-in", "admin command"

**agent task**: a command that is conversation with the agent — it joins the chat's accumulation as a command entry, so it survives a crash, and is answered in the conversation (`/organizememory`, `/explain`, `/digest`).
_Avoid_: "task" (a scheduled task is `WakeReason::Scheduled`)

**host copy**: a reply the host composes itself instead of the agent, in the chat's language — keys and strings are `agent/copy.rs:HostCopy` and its `render`.
_Avoid_: "template", "i18n string"

**Skill**: a specialist capability described by a SKILL.md file; the prompt lists the names and the body is loaded with `read skill://<name>`.
_Avoid_: "plugin"

### Platform domain

**channel**: one configured platform instance and its in-process runtime — the agent's presence on one outward surface, with its identity (`ChannelProfile`, `ChannelKey`), what that surface supports (`ChannelCapabilities`), and the way in and out (`agent/channel.rs:Channel` out, `ChannelAccess` in). One platform implements one `ChannelAdapter` per instance, and the agent layer holds the outbound `Channel` and never a platform type.
_Avoid_: "ChatType::Channel" (a chat kind, not this); "bot" where the thing is literally a bot; conflating with ChatAgent or with the platform itself

**channel handle**: one chat on that channel, as the agent acts on it (`agent/channel.rs:ChannelHandle`) — the chat is bound where the agent is created, so a send, a typing indicator or a platform file already know which chat they mean and no caller pairs a chat id with a channel. What the chat itself does not decide (`key`, `profile`, `capabilities`) it forwards to the channel.
_Avoid_: "guard" (a guard's meaning is its lifetime — that is `TypingGuard`); "handle" for the channel itself, which is `Channel`

**channel key**: the `platform:key` spelling one channel instance is stored under — prepared once where the channel is built, read by the scheduled-task scope, the log labels and nothing else. An identity travels as a typed value; the spelling exists for the columns and the logs.
_Avoid_: "channel id" (that is the platform's own instance identity), "instance name"

**platform declaration**: what one platform offers the agent (`agent/channel.rs:ChannelCapabilities`) — the forms it can send and what each form takes, whether a message may point at another one or quote a part of it, and the upload ceiling. The code that must vary with the platform reads the declaration; a difference with no reader is not declared.
_Avoid_: "capability list" (only the forms are a list), "platform config" (that is the `[channel.<name>]` section)

**channel adapter**: one platform's own half of the platform layer (`platform/adapter.rs:ChannelAdapter`) — the key its channel is stored under, its **platform declaration**, reading its updates into an **arrival**, sending what the agent asked for and answering with a receipt (`platform/adapter.rs:SentMessage`), starting a **typing guard**, and file access. Only the platform layer names it; the agent speaks **channel** on the way out and **channel access** on the way in.
_Avoid_: "transport" (that is HTTP or the vendor's client), "wire" (our own word for it), "adapter" for the layer as a whole

**arrival**: one update a platform read off its own update stream and handed to the inbound tail (`agent/intake.rs:Arrival`) — chat (external id, kind, title), sender, text, attachments, external id, reply-to, the change kind with the platform's own times, who it was aimed at, the platform's own message meta, and what the platform made of the text (`Ordinary`, its own `Greeting`, or a `Command` with its name and payload). Platform ids are strings here; the tail resolves them to rows. It carries no platform-shaped field of its own: whatever a platform knows beyond this set rides in its `meta`.
_Avoid_: "update" (that is the platform's own payload), "event" (that is the agent's audit trail)

**channel access**: the inbound path's narrow handle on the platform (`agent/channel.rs:ChannelAccess`) — whether a chat may talk to this channel, and how to deliver host copy. The adapter answers it (`platform/adapter.rs:ChannelAdapter` declares it as a supertrait), so the tail reads two methods and no vendor type.
_Avoid_: "channel" bare when only the inbound half is meant

**typing guard**: the live typing indicator, held as a value whose lifetime is the indicator's (`agent/channel.rs:TypingGuard`) — dropping it ends what the platform started. How it is kept alive between those two points is the platform's fact, so the agent never names a cadence; a platform with no indicator answers `None`.
_Avoid_: "typing flag" (nothing declares whether a platform has an indicator — it answers with a guard or with nothing)

**inbound tail**: the one path an **arrival** takes after a platform hands it over (`agent/intake.rs:Intake`) — admission, the command fork, resolving the chat and its account, one-transaction ingest, and the doorbell. It is agent-owned: everything it reaches for is agent or domain vocabulary, so a platform only reads its own updates into an arrival and hands it over.
_Avoid_: "dispatcher" (that is one platform's routing), "platform intake" (the tail is not the platform layer's)

**ChannelHost**: the process-level host owning every channel — it starts them and orchestrates shutdown. What it keeps per channel is `platform/host.rs:ChannelRuntime`: the channel's `ChannelContext` (the adapter, the one inbound path, the chats' agents, and the token their shutdown hangs on) and the supervisor handle it stops.

**AppContext**: the process-level container holding config, settings, database services, the chat transport, multimodal and the event bus (`agent/runtime/context.rs`).

**MCP**: Model Context Protocol, the external tool-server protocol.

## Relationships

- One **Chat** owns several **Topic** and several **Message**; a **Topic** consists of several **note**
- One **Chat** owns exactly one **ChatAgent** (created lazily) and one **ConversationRecord**
- One **ChatAgent** owns several **Turn**; one **Turn** contains several **Step** and produces several **Event**
- One **ChatAgent** may run several **chapter wrap-up**, each opening a new **Chapter**
- One **Account** owns several **Memory**; a **Memory** belongs to one **Chat** and may be attributed to one **Account**
- One **KnowledgeBase** consists of several **KnowledgeDocument**, each of several **KnowledgeChunk**, each carrying one **Collection** label
- One **ChannelHost** owns several **channel**, each owning one **AgentRegistry**

## Example dialogue

> **Dev:** "A message arrives and the ChatAgent should wake — should the wake carry the message content?"
> **Domain expert:** "The change is written down, not carried: one transaction writes the message row and appends a ChatEntry to the chat's accumulation. What reaches the agent is a doorbell, and the agent re-reads the accumulation — one copy of the state, and only one thing to keep in step."
> **Dev:** "So what counts as one LLM call inside a Turn?"
> **Domain expert:** "That is a Step. A Turn is the whole loop — several Steps plus tool calls, until the agent stops. Calling them 'rounds' misleads the scheduler."
> **Dev:** "And those persisted TurnStarted/ToolCall records — run logs?"
> **Domain expert:** "Those are Events: observable agent-run events with a typed payload, consumed for auditing and recovery. Logs are tracing output."

## Flagged ambiguities

- **Chat** is a platform conversation, **ChatAgent** is the agent entity running in it, and **channel** is a configured platform instance — three layers, never interchangeable
- "summary" is a normal reply; the **chapter wrap-up** produces a retention summary that replaces history, and that summary is the next chapter's opening — a **Memory** is a separate stored entry, never the retention summary
- **AgentRole**, **ModelRole** and "persona" are three different things; never write bare "role"
- "knowledge" is ambiguous: a memory entry of kind knowledge is chat-scoped, while a **KnowledgeDocument** belongs to the global corpus
- Three unrelated retrievals: **WebSearch** (live web), knowledge-base search (curated corpus), **recall** (chat-scoped memory and topics) — name the tool, never bare "search"
- **need-close** is rendering semantics, and a closed topic stays editable
- "context" has four senses: the process container (`AppContext`), the per-turn carriers (`TurnContext`, `ToolContext`), and the prompt-rendering package — the per-turn carrier module is `turn`, not `context`
