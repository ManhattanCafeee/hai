# CLI / event log TUI / rebuild

> `hai`'s command surface: the management families, the destructive embedding rebuild, and the event-log TUI. Read before changing `cli.rs`, `cli/`, `cli/tui/`, `domain/db.rs` or `domain/repo/vector.rs`.

## Overview

One binary, two modes: with no subcommand it starts the bot (`app/mod.rs:serve`), otherwise it runs one `config` / `db` / `log` / `kb` management command, assembled by `cli.rs:Cli::execute`, which loads `.env`, the config, then dispatches — a failure exits 1 after printing the error (`main.rs:main`). A management command never starts the platform layer (`log` and `kb` build their own pool), and `[model-roles.embedding]` is required for `db migrate`, `kb` and the rebuild (`docs/topics/config.md`) — the rebuild resolves that role alone, so it needs no chat model; the flags are the `clap` definitions behind `--help`.

## Design

### db

`create` rewrites the configured URL onto the `postgres` database (`domain/db.rs:create_database`) and issues `CREATE DATABASE`. `migrate` is one call into `domain/db.rs:migrate`, which runs `domain/db.rs:apply_schema` — `crates/hai/migrations/schema.sql`, then `domain/db.rs:normalize_legacy_columns`, which backfills legacy NULL `memory.source_message_ids` to `'{}'` and makes the column `NOT NULL DEFAULT '{}'` while dropping the stale DDL defaults the write paths now bind (`memory.status`, `account.last_active_at`, `topic.started_at`, `topic.last_active_at`) — and then `domain/repo/vector.rs:ensure_embedding_schema`, which adds the vector extension and the `embedding` column at the configured dimension. Nothing runs automatically, so a deploy must run this.

### rebuild embeddings (`cli/rebuild.rs:rebuild_embeddings`)

`domain/repo/vector.rs:reset_embedding_dimension` → `domain/repo/vector.rs:collect_embedding_jobs` → `run_batch` → `domain/repo/vector.rs:rebuild_embedding_indexes`: a destructive full rebuild rather than an incremental fill — `reset_embedding_dimension` drops and recreates every `embedding` column at the configured width (`domain/repo/vector.rs:EmbeddingTable`, whose arms are also the table list the jobs come from), discarding the stored vectors and the per-table index, before re-embedding every memory, perception and knowledge chunk plus topics with a non-empty summary. Writes are per row (`domain/repo/vector.rs:upsert_embedding_vec`): a failed row keeps a NULL embedding retrieval skips while successes stay committed, and counted failures end the run non-zero. The index is rebuilt last, since IVFFlat needs rows to build on.

### kb (`cli/kb.rs:execute`)

`import` / `list` / `search` / `delete` / `reindex`; the CLI supplies files, collection and title, while content-hash idempotency, the chunker and `reindex`'s stale-version bypass belong to `docs/topics/domain.md`. Rules the flags do not express:

- A directory is an error unless `--recursive` is given, and so is a file that is not `.md`/`.txt`.
- `search` prints `p=` (the reranker's score) when reranking is configured, `d=` (the vector leg's L2 distance) otherwise, and `-` for a chunk only the lexical leg recalled.
- A `---` block is frontmatter only when it holds a `key: value` line, so a document opening with a horizontal rule keeps its body; the title comes from that block, otherwise the file stem, and `--title` overrides both.

### log (`cli/log.rs`)

`--id N` prints one event in full through the same section renderer the TUI uses (`cli/display.rs:render_sections`); otherwise `--chat` and `--event` seed the TUI's query line as `chat:<id>` and `kind:<name>`, which from there on governs the window.

### Event log TUI

Three panes over one window — the chats the window holds, the timeline of turns, and the detail of one event — plus a query line that is not one of them: `/` opens it for as long as the reader is typing, `Enter`/`Tab` keep the terms and give the keyboard back, and `Esc` puts back the terms that were in effect when it opened. The detail shares the body with the timeline while it is open and takes all of it in fullscreen (`cli/tui/detail.rs:DetailLayout`). The key map is per mode: `cli/tui/action.rs:key_event` reads a key into a `cli/tui/action.rs:Action`, and `cli/tui/app.rs:App::update` applies it and returns the `cli/tui/action.rs:Effect`s the database has to run. Memory-only work happens where it was asked for; database-bound work runs in `cli/tui/app.rs:App::apply`, which keeps I/O off the key path.

The keys, by mode — `cli/tui/action.rs:key_event` is the one place a key becomes an action:

|where|keys|
|---|---|
|any pane|`Tab`/`Shift-Tab` or `h`/`l` cycle the panes, `/` opens the query line, `?` help, `q`/`Ctrl-C` quit, `Esc` cancel|
|chats|`j`/`k` move, `Enter` narrows the window to that chat, and again to widen it|
|timeline|`j`/`k` move, `Enter` opens a turn or an event, `PageUp`/`PageDown` and `Ctrl+U`/`Ctrl+D` page, `g` the oldest loaded row and again for the page before it, `G` the newest and follow, `f` follow|
|detail|`j`/`k` scroll, `PageUp`/`PageDown` and `Ctrl+U`/`Ctrl+D` scroll further, `Enter` shows every long field, `f` fullscreen, `Esc` closes it|
|query line|types the terms; `Enter`/`Tab` keep them and close, `Esc` puts back the ones that were in effect|

The timeline's unit is a turn: `cli/tui/event_store.rs:group_turns` groups the window into runs of one chat and turn number, a turn is one row that opens into the events inside it, and a record carrying no turn of its own — a wrap-up — stands alone. A selection names the row's event *and* whether it is a child row (`cli/tui/timeline.rs:RowKey`), because an open turn's first event is both its header and one of its children; `cli/tui/timeline.rs:clamp_offset` keeps the selected row in the upper third of the viewport, and a prepend restores its visual position so loading older events does not jump. A prepend can also hand a turn events that come before the one the reader was on, which moves that turn's own first row: the selection follows its event to the turn that holds it (`cli/tui/timeline.rs:Timeline::reselect`) instead of falling back to the head, where it would pull the next page in on the same tick. The window is a seq-ascending slice of matching events and only grows — `extend_back` prepends, `append_new` appends, neither trims — because trimming would evict browsed history and invalidate the selection; only the initial load trims to capacity, which `cli/tui/event_store.rs:set_viewport` derives from the viewport height.

The query line is the filter. `chat:` and `kind:` are the terms the database can narrow by and are re-read as soon as they change (`cli/tui/app.rs:App::update` compares them with what the window was queried with); `tool:` (a glob), `turn:`, `slow:`, `err` and bare text are matched against the rows already in memory (`cli/tui/filter.rs:Terms::matches`). A query that does not parse leaves the last one that did in effect, so a half-typed value never disturbs the window; the header spells out what is in effect, which is not always what was typed — a token the grammar does not know is a search, not a filter.

Long fields fold: `cli/display.rs:Section` holds one field's lines, and the detail shows the first `cli/display.rs:PREVIEW_LINES` behind a `▸` plus a marker saying how much is hidden, until `Enter` unfolds every section.

New events are counted and offered (`G` loads them) unless the reader is following the tail (`f`), where the tick appends them; with the selection parked at the oldest loaded row the tick pulls in the page before it.

## Boundaries

- The event log is read-only — no editing, deletion, export or retention policy — and the filters select one chat.

## Trade-offs and Reasons

- **Only a term that changes the *set* of events goes back to the database.** The window is already in memory, so a keystroke refilters it (case-insensitive over the one-liner, seq and chat id) with no round trip; `chat:` and `kind:` change what the query returns, so they are re-read at once instead of on a timer.
- **A turn's summary is its start record's line.** The header row reuses that rendering rather than parsing the reason and the message count back out of it, and a turn whose start is older than the window says so instead of passing a child off as the turn.
- **Following lands on the newest turn, not on the newest record.** A stream of wrap-ups would otherwise pull the reader off the last thing that happened, so a record with no turn of its own never takes the selection.

## Pitfalls / common mistakes

- **`event.payload` is JSONB**, filtered as `(payload->>'chat_id')::bigint` and `payload->'payload'->>'event'`; the expression indexes in `crates/hai/migrations/schema.sql` must spell them identically or the query stops using them, and the chat id's explicit `::bigint` cast is what matches the numeric index.
- **`$N` placeholders are numbered by hand** in `domain/repo/event.rs:push_filters` and the two statement builders that call it (`domain/repo/event.rs:query_sql`, `:count_after_sql`), over raw SQL under `sqlx::AssertSqlSafe`, so a mis-numbered bind fails at runtime.
- **Condition fragments must keep their leading space**: the base statement ends in `WHERE seq > 0`, so an appended fragment has to start with `" AND …"`.
- **`db migrate` does not create the vector index**, so without a rebuild retrieval scans instead of using IVFFlat — and that rebuild nulls every embedding first.
- **A `kind:` term is coarser than its label**: one stored tag covers every `turn-ended` outcome, so `kind:turn-ended` also shows the turn that failed (`docs/architecture.md`).
- **A query term is a key only where the grammar says so**: `slow` alone is a search for that word, `slow:abc` is a refusal — and the header spells out which one the reader is looking at.
- **A turn number restarts with every chapter** (`agent/runtime/chat/conversation.rs:Conversation::turn_count`), so `(chat, turn)` names several turns over a log's life: a `turn:` term matches each chapter's turn of that number, and the TUI remembers an opened turn by its own first record instead (`cli/tui/event_store.rs:TurnGroup::anchor`).
- **Input waits first, the log polls on a timeout** (`cli/tui/guard.rs:InputReader` plus the timeout in `cli/tui/app.rs:App::run`): a `select!` that races input against a timer drains a key burst at the timer's pace, which is 230 ms per key against 0.9 ms once input comes first — and it also makes a single keypress wait for the timer.

## Evolution Direction

- A turn's outcome is read from the rendered payload, so filtering by one (`err`, `slow:`) only sees the window; doing it in the query would need a filterable discriminator in storage.
