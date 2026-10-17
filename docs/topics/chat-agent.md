# ChatAgent lifecycle and scheduling

> How a wake becomes a Turn, how a Turn ends, and when the chapter reopens or the environment exits. Read before changing `agent/runtime/chat/`, `agent/runtime/event/` or `runtime/engine.rs`. Terms: `docs/CONTEXT.md`; the wake-to-reply sequence is `docs/architecture.md`.

## Overview

One Chat maps to one ChatAgent; this document owns *when a Turn is dispatched and the chapter lifecycle*, the Step loop being `agent/runtime/agent_loop.rs`.

## Design

### States

`agent/runtime/chat/mod.rs:ChatAgentState` is exactly `Idle | Busy { result_rx, started_at }`, and **wrap-up is one flavour of Busy** (`agent/runtime/types.rs:BusySignal` distinguishes them).

```mermaid
stateDiagram-v2
    direction LR
    [*] --> Idle
    Idle --> Busy: dispatch (scheduler Ready) / reopen (wrap-up)
    Busy --> Idle: Turn / Steered / Wrap-Up, a failure, a cancellation, or a panicked task
    Idle --> [*]: idle expiry with an empty chapter, or shutdown
```

### Turn outcomes

A Turn is atomic, and only a normal end advances state. **Success** (the agent stops: `skip`, no more calls, or the Step cap) advances context messages, `ContextMeta`, the consumption of the batch it took and the snapshot together; **Steered** (something arrived mid-Turn) advances the same and starts a new Turn at once; **Failed** (a loop error) advances nothing.

Nothing is partial, so the batch a failed Turn took is still in the accumulation. That is what arms up to two bounded re-dispatches per batch (`agent/runtime/chat/mod.rs:MAX_TURN_RETRIES` — three attempts in total, reset by any success or by a fresh change; the attempts left and the instant the owed attempt may go live in `agent/runtime/chat/mod.rs:RetryBudget`, cleared where the attempt is dispatched rather than where it is read, and read ahead of the attention rules by `agent/runtime/chat/event_loop.rs:ChatAgent::idle_tick`) — otherwise the reply would vanish silently and an unstarted chapter would exit on idle; a failure before the spawn (gathering, prompt build) leaves the batch untouched, counts against the same budget and backs off for a second before the next attempt (`agent/runtime/chat/mod.rs:ChatAgent::stand_down`), and a spent budget stands the batch down until a new change reopens it.

### Steering

Steering (`agent/runtime/agent_loop.rs:run_loop`, enabled per role by `agent/role/mod.rs:AgentRole::spec`, Default taking `[agent.context] steering`) continues attention instead of preempting it: a Step ending with an arrival since the Turn started ends the Turn as Steered — the doorbell (`agent/runtime/event/doorbell.rs:Doorbell::take_signal`) is what the loop checks — what it processed takes effect, and a new Turn starts at once over the accumulation as it now stands, so it reads whatever arrived rather than only what the old Turn had gathered. WrapUp sets `steering = false`, so arrivals wait for after the pass.

### Scheduler

`agent/runtime/chat/scheduler.rs:EventScheduler` with `agent/runtime/chat/attention.rs:Heat`/`Window` is a pure timing engine, free of the database and the platform. It holds no changes of its own: `ChatAgent::refresh_batch` reads the accumulation, hands each entry it has not seen yet to `EventScheduler::on_event` by its `domain::vo::EntryAudience` — addressed (`Agent`) or a bystander (`Bystanders`) — and `decide` is told whether anything is pending at all. That split drives the rules:

- **Debounce** — a bystander change arms a quiet period, merging a burst into one Turn.
- **Heat** — pending changes plus a lucky roll dispatch probabilistically, letting random attention speak unprompted; nothing pending never dispatches.
- **Attention window** — being addressed, or having spoken itself, opens it and resets heat to maximum; everything pending inside it dispatches. The two openers are named so the paths stay apart (`agent/runtime/chat/scheduler.rs:EventScheduler::on_addressed`, `:on_spoke`): a change aimed at the agent also drops a quiet period already running, while the agent's own reply (`agent/runtime/chat/dispatch.rs:ChatAgent::on_complete` → `on_spoke`) leaves that quiet period standing — what arrives inside the attention window after a reply dispatches, an overheard burst keeps its debounce.
- **Idle baseline** — only the end of a run moves it (`ChatAgent::end_run` → `EventScheduler::mark_run_end`); no arrival and no attention window does, so a chat that keeps receiving ignored messages still times out.
- **Expiry** — past the idle baseline plus `idle_timeout` the answer is `Done`: end the idle state, never dispatch a backlog whose context is long gone. A backlog is dropped, not lost: the changes stay in the accumulation, and a later Turn reads them.

The Turn that results records the ground it was dispatched on, the batch it read, and the strongest reason that batch carried (`domain/vo/event.rs:AgentEventPayload::TurnStarted`, `started_by`, `composition` and `reason`; `StartedBy` also carries `Retry` and `Steered`, the two paths that never reach the scheduler). A batch reason alone cannot tell an overheard batch heat let through from one an addressed change opened the attention window for; these two fields can, and they are what `hai log` shows.

### Scheduled wakes

`agent/runtime/task_watcher.rs:spawn_task_watcher` appends a scheduled entry to the chat's accumulation and rings the agent, exactly as a platform change does; a recurring task advances to its next trigger point, so missed occurrences collapse into one entry, and a one-off task is deactivated (`docs/topics/tools.md`).

### Reopen and wrap-up

The chapter reopens **whether or not the wrap-up succeeds**: the pass runs once, and on success its summary opens the next chapter. At idle, before anything dispatches, two triggers are checked (`agent/runtime/chat/event_loop.rs:ChatAgent::idle_tick`): the **token threshold** (the chapter's tokens reach `[agent.context] compact-token-threshold`, 0 disables, checked first so the context cannot outgrow the model window) and **idle expiry** — no run for `[agent.context] session-idle-timeout-secs`, measured from the ChatAgent's last return to Idle, so the attention window plays no part in the deadline (unwrapped content, `ContextMeta.turn_count > 0`, reset by the reopen so it cannot fire twice per chapter; an already-overdue restored agent reopens on its first tick).

The pass (`agent/runtime/engine.rs:AgentEngine::spawn_wrap_up`) is a Turn numbered 0 with the WrapUp role; its retention instruction is appended **as a user message**, its system prompt byte-identical to the Default role's (`agent/role/mod.rs:AgentRole::spec`). It stops on the first Step with no tool call (`agent/role/wrap_up.rs:decide`), so the model may organize across Steps.

- **Success** — emit `WrapUpCompleted` (the wrapped chapter's Step count, the pass's own context tokens, the summary), open the new chapter with the summary at its head, reset `ContextMeta`; the accumulation is untouched, being conversation-level while the context is chapter-level.
- **Failure** — emit `WrapUpFailed` and open a clean chapter; no retry, no backoff.
- **Extraction** — machine, not conversational: the most recent Step carrying a `<summary>` tag, else the most recent non-empty Step text; only an all-empty pass fails.

### ContextMeta

`domain/vo/context_meta.rs:ContextMeta` holds the last successful Turn's context tokens (the over-limit criterion), the chapter's successful Turn count (what "unwrapped content" means) and its Step count (display and the `WrapUpCompleted` payload); Turns are never persisted, and auditing rides on the event table.

### First turn

`agent/runtime/chat/conversation.rs:Conversation::is_first_render` is `turn_count == 0`: a chapter with no completed Turn is built whole — a new conversation and either wrap-up outcome alike, the summary a chapter opens with not counting as a Turn — while a chapter that has run one continues incrementally. A failed first Turn stays eligible, failure never advancing the counter.

### The batch

A Turn reads the accumulation through `ChatAgent::gather_batch`: the messages its entries name, deduplicated by message id (a message created and edited in one batch renders once, as the projection now holds it), plus — on a chapter's first render only — the recent history up to `[agent.context] context-seed-cap` that gives them a setting (`domain/service/message.rs:MessageService::recent_asc`). Nothing else about "what is new" is stored: the divider in the rendered conversation comes from the batch's message ids, so a Turn that read nothing has nothing to divide.

Consumption happens once, at a normal end, and in the same statement as the snapshot (`ChatAgent::on_complete` → `ConversationRecordRepo::save`): one insert-or-update, so a first save cannot race the ingest that appends to the same row; the write drops exactly the batch's entry ids, relative to what the accumulation holds by then, so a context and the changes it read can never disagree and an entry appended while the Turn ran stays for the next one. A Failed Turn consumes nothing, which is why the same batch comes back on the retry.

## Boundaries

- ChatAgents are rebuildable; conversations are permanent state.
- Reopen has exactly two triggers and no startup scan: an overdue but idle chat reopens on its next wake.
- A generation start re-feeds (`agent/runtime/startup_refeed.rs:spawn_startup_refeed`) every chat whose accumulation is unconsumed: a doorbell per chat, never a forced Turn, so an addressed change opens the attention window while a bystander one still waits for heat.
- Exit needs an idle expiry with an empty chapter or the shutdown signal. A Busy agent is cut by that same signal: the run selects on the chat's token (`agent/runtime/engine.rs:AgentEngine::spawn_busy`), reports `BusySignal::Cancelled`, and consumes nothing, so what it read comes back if the process starts again.
- Turn numbering is chapter-level; a wrap-up takes none.

## Trade-offs and Reasons

- **Reopening never depends on the wrap-up**: retention is a quality feature and must not block availability, so a failed pass opens a clean chapter instead of retrying.
- **The wrap-up shares the Default prompt and tool surface**: a byte-identical prefix preserves the provider cache, and rejecting `send_*`/`skip` at runtime costs less than a second prompt.
- **A steered Turn keeps its input intact**, so its outcome is applied or discarded whole.
- **Failure discards state instead of repairing it**: consuming only on success is what lets the next Turn retry with no repair path.

## Pitfalls / common mistakes

- **Turn and wrap-up failures differ** (`BusySignal::TurnFailed` vs `WrapUpFailed`): the first keeps Idle with the batch untouched, the second opens a clean chapter. A `Cancelled` run is neither: it ends the run and leaves the state, including a half-wrapped chapter, exactly as it was.
- **`should_wrap_up` asks for unwrapped content**, not message counts; changing it must respect both boundaries (metadata resets, the accumulation preserved).
- **Steering belongs to the Default role only** — interrupt handling in the WrapUp role would break retention.
- **"The agent disappeared" has two causes**: an idle exit with an empty chapter (dropped at once, rebuilt on the next wake) and a creation failure (logged, no agent built).

## Evolution Direction

- **Unread fetch cap**: a long-offline pile-up can overflow the context; visibility wins for now, truncation by priority is the direction (`docs/architecture.md` owns the startup scan).
