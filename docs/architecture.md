# architecture

> Layering, the channel traits, the event system, the module map. Read before touching cross-module wiring, dependency direction or event persistence.

## Architecture declaration

Layered top-down — `app → platform → agent → agentcore + config + domain`, over `util` / `error` — with dependencies pointing downward. `agentcore` is a core library with no `agent`/`domain`/`config` dependencies, `config` is pure data, `domain` is the data layer, and no platform detail reaches the agent layer except through the channel traits (`agent/channel.rs:Channel` outbound, `ChannelAccess` inbound). No cycle remains: every edge runs downward. Adding an upward dependency is a defect.

### Measured dependencies

```txt
app → platform → agent → agentcore + domain + config
                     ↘  domain → agentcore (the embedding and rerank traits, plus `Language`)
config → agentcore (Language, ModelKind, ProviderKind, Credential, ModelTarget, mcp::McpConfig)
domain → config (the database block) + error
agentcore → util (text) + error
platform → agent + agentcore + config + domain + error
cli → app + agent + agentcore + config + domain + error
error.rs → nothing internal
```

- **`agentcore` names no config type**: the MCP block's own types (`agentcore/mcp.rs:McpConfig`, `McpTransport`) live with the client that reads them, and `agentcore/mcp.rs:McpManager::from_config` takes the `[mcp]` map instead of the whole `AppConfig`, so every cross-edge above is downward — which is what a crate split would need.

Nothing else points upward: the process container lives in `agent/runtime/context.rs:AppContext` — the lowest layer that can name every one of its parts (config, domain services, agent settings and the event bus) — so only the composition root enters it (`app/mod.rs:serve` for the bot, `cli.rs:Cli::execute` for a management command).

`agentcore` referencing `crate::agent` or `crate::domain` is the one hard prohibition, and it holds today.

### The channel traits

Two agent-owned traits, the handle the agent acts through, the platform layer's own adapter contract, and two shared pieces:

- `agent/channel.rs:Channel` — what the platform layer implements for one configured channel: its key, its profile, its declaration, and the verbs that act on a chat (send something, start the typing indicator, read a platform file). The agent acts through `agent/channel.rs:ChannelHandle`, which binds one chat to that channel, so no caller re-pairs a chat id with a channel; the outbound vocabulary is `ReplyTo` (where in the chat: topic, reference, quote — where `reference` is an agent-side message id and `quote` narrows it to a part, never a platform value) and `OutboundMessage` (that plus the chat and the payload). A send's receipt (`platform/adapter.rs:SentMessage`/`SentAttachment`) and the adapter's own message types belong to the platform layer, because that layer is their only reader. What the renderer produces is the renderer's own type (`agent/context/types.rs:BuiltContext`).
- `agent/channel.rs:ChannelAccess` — the inbound half: whether a chat may talk to this channel and how to deliver host copy. `platform/adapter.rs:ChannelAdapter` declares it as a supertrait, so the adapter the tail receives provably satisfies it, and no forwarding code exists.
- `platform/adapter.rs:ChannelAdapter` — the platform layer's own contract, implemented once per platform and named by no agent code.
- `platform/outbound.rs:OutboundRecorder` — implements `Channel` over a `ChannelAdapter`: it resolves an internal target to the platform's address, holds the reference to the conversational rules, and records the row that says the message left. An adapter reads no domain rows and holds no process container.
- `agent/intake.rs:Intake` — the inbound tail, agent-owned because everything it reaches for is agent or domain vocabulary; a platform only reads its own updates into `agent/intake.rs:Arrival` and hands it over.
- `platform/host.rs:{ChannelContext, ChannelRuntime}` — one channel as the host holds it: the adapter, the one inbound path, the agents living in its chats, and the token their shutdown hangs on; `ChannelRuntime::assemble` builds all of it from a platform's adapter, so that part is written once.

**The declaration carries only what a reader varies with.** The send tool derives its advertised schema and its runtime validation from `ChannelCapabilities`, and that is what it holds. A field with no reader is not added — a platform's ability to deliver edits, say, is exercised by that platform's arrival mapping, not declared, and whether it shows a typing indicator is not declared either: the platform answers `Channel::start_typing` with the guard or with nothing (`agent/channel.rs:TypingGuard`).

**Platform autonomy**: the agent boundary carries neutral semantics (`ChatType`, `EntryAudience`, the message text and its attachments), and the store keeps that neutral shape — a platform adapter reads its own update into it on the way in (`Arrival`, whose own fields are platform-free: extras ride in `meta`) and reports the same shape on the way out (`SentMessage`). `domain` may carry platform *variants*, but no platform crate types.

## Key data flows

### Platform event to persisted events

```mermaid
flowchart LR
    D[inbound tail<br/>row + entry, one transaction] -->|1| DB[(PostgreSQL)]
    D -->|2 doorbell| I[agent Doorbell]
    I -->|3 re-read the accumulation| S[scheduler: timing decision]
    S -->|4 Ready| P[dispatch: take the batch, build prompt]
    P -->|5 spawn turn| L[Step loop: LLM + tools]
    L -->|6 consume the batch| DB
    L -->|7 events| BUS[event bus]
    BUS -->|8 batched| DB
```

The sequence carries the design: **a change is durable before anything is rung** (the row and its accumulation entry land together, so the agent always reads one state); **the scheduler only times** (it owns no side effects and never reads the database); **a Turn is the atomic unit** (the batch it took is consumed only if it ends normally, so a failure needs no repair); and **events are batched out of band**, so auditing never blocks a reply.

### Event system

- `domain/vo/event.rs:AgentEventPayload` is the typed payload enum (serde tag `event`, kebab-case names); `AgentEvent` adds the chat id and serializes into the table.
- `agent/runtime/event/bus.rs:AgentEventBus` buffers emissions and flushes them in batches into the `event` table under the domain `agent`. The table has no chat or kind column: both live inside the JSONB payload, which is what the query indexes are built on.
- Two Event semantics matter beyond their names. `ModelRetry{ResponseWithText}` is **not** a retry — it records a speaking-contract violation and the loop continues; only timeouts and unparsable responses are actually retried. And the wrap-up events bracket the retention pass, with `WrapUpCompleted` carrying the wrapped chapter's Step count, the pass's own context tokens and its summary.
- `cli/display.rs:tag_for_kind` names the display tag of a payload — an exhaustive `match`, so a new variant fails to compile; the TUI and the `--kind` filter read the same `EventKind::as_str()`, so there is no second hand-written map to keep in step.

### Two HTTP clients

| Path | Client | Why |
|---|---|---|
| Main conversation | genai (`agentcore/provider.rs:create_genai_client`) | multi-provider adaptation, including the adapters for OpenAI-compatible vendors |
| Embedding, speech, multimodal, image generation, rerank | `agentcore::ApiClient` | a small REST client for the tooling endpoints, under the project's own control |

### Errors

One `AppError` enum, no codes or families, boundaries consuming the display string — the shape and its conventions are in `docs/topics/config.md`.

## Module map

| Module | Responsibility |
|---|---|
| app/ | composition root: logging, bot startup and shutdown orchestration |
| platform/ | channel lifecycle host and what it keeps per channel (`platform/host.rs:ChannelContext`), the outbound recorder, and each platform's adapter (the Telegram module: dispatcher, arrival mapping — the only place an update becomes an `Arrival` — command spelling, menu, config, wire impl, forms, receipts, reply resolution, file access, send service) |
| agent/ | ChatAgent and scheduling, the Step loop, roles, context rendering, the command table and host copy, tools, multimodal, the analysis pipeline, the sandbox, networking — and the process container (`agent/runtime/context.rs:AppContext`) |
| agentcore/ | core library: tool abstraction, MCP, provider clients, the model-kind table (which protocol each kind speaks), skills, XML rendering |
| domain/ | data layer: models, repos (including the runtime embedding columns), services, value objects, the schema |
| config/ | pure data configuration, file and environment merging, the `[model-roles]` table, provider resolution |
| cli/ | command surface and the event-log TUI |
| util/ error.rs | text helpers and the file cache, the single error type |

## Evolution Direction

- **Startup scan**: reopen chapters that are overdue while idle, with throttling (the interface is left open today).
- **New platform**: a `platform/<name>/` submodule whose adapter implements `platform/adapter.rs:ChannelAdapter` — which includes the inbound half (`agent/channel.rs:ChannelAccess`), so no glue is needed — its declaration, the update stream read into `agent/intake.rs:Arrival`, the send calls answering with a `platform/adapter.rs:SentMessage`, host copy, and file access — plus its own config struct parsed from its `[channel.<name>]` section; one arm in `platform/host.rs:ChannelHost::spawn_all`; and one variant in `domain/model/account.rs:Platform`. Nothing in `agent/` and nothing else in `domain/` changes: admission, the command fork, ingest and the doorbell are the shared tail's (`agent/intake.rs:Intake`), and the sent row is the shared recorder's.
- **Crate split**: not now — one process and no external consumer of the library surface. Revisit when compile time degrades or a second process needs `domain`/`agentcore` at crate granularity.
- **Deliberately deferred, not to be fixed opportunistically**: the event tag spellings living in three places; a row and its vector still written non-atomically on the perception path (it is a cache and that write is best-effort by design, `docs/topics/domain.md`); and the hand-written JSON keys in the conversation snapshot.
