# HAI

## Project overview

HAI is a long-term companion AI assistant running as a Telegram bot. Rust workspace (`hai` + `hai-macros`), layered `app → platform → agent → agentcore + config + domain` over `util` / `error`. The `docs/` tree is the spec authority; `docs/CONTEXT.md` is the term authority and `docs/architecture.md` holds the dependency rules.

Terms that are easy to get wrong (definitions in `docs/CONTEXT.md`):

- a **Turn** is one loop execution, a **Step** is one LLM call
- the **chapter wrap-up** is the chapter-end retention pass; a "summary" is a normal reply
- **need-close** is a rendering hint, not a stored state
- a **ChatEntry** is one accumulated change (the reason a Turn reads is derived from it, never carried by the doorbell)
- a ChatAgent is **Idle or Busy** — wrapping up is a Busy

## Commands

### Toolchain

- **Toolchain**: `rust-toolchain.toml` pins nightly (edition 2024)
- **Build**: Cargo workspace (members `crates/*`, resolver 3) — `cargo build`; one crate: `cargo build --manifest-path crates/hai/Cargo.toml`
- **Check**: `cargo clippy --all-targets` must be warning-free before delivery (the only tolerated note is the external `proc-macro-error2` future-incompat warning)
- **Format**: `rustfmt.toml` (`imports_granularity = "Crate"`, `group_imports = "StdExternalCrate"`, `reorder_imports = true`)
- **Schema**: `hai db migrate` is `domain/db.rs:migrate`, which runs `domain/db.rs:apply_schema` (which applies `crates/hai/migrations/schema.sql` and then `normalize_legacy_columns`: legacy NULL arrays become the empty array the model reads, and stale DDL defaults are dropped) and then `domain/repo/vector.rs:ensure_embedding_schema`; it never runs automatically, and only `hai db rebuild embeddings` builds the vector index

### Project commands

| Command | Purpose |
|---|---|
| `cargo run --bin hai` | no subcommand → start the bot |
| `hai config [--format json\|toml]` | print the loaded configuration |
| `hai db create` | create the database |
| `hai db migrate` | create tables, columns, query indexes and constraints from the schema |
| `hai db rebuild embeddings` | rebuild every vector and its index |
| `hai log [--id N] [--chat ID] [--event TAG]` | event-log TUI; `--id N` prints one event |
| `hai kb import/list/search/delete/reindex` | knowledge-base management (idempotent import, semantic search, rebuild) |

## Code style

<CRITICAL>

- **Terms** follow `docs/CONTEXT.md`; never invent a second name for a concept that has one
- **Layering holds**: `agentcore/` must not depend on `agent/` or `domain/`. The known cycles are listed in `docs/architecture.md`; add no new ones
- **Types over conventions**: domain identifiers are newtypes, fixed sets are enums, events are the typed payload enum — no bare identifier strings and no magic values across a boundary
- **Missing data violates the contract**: fail fast, never substitute a fallback, and do not spread the legacy "swallow the error and return `Ok(None)`" pattern
- **Enum wire form lives once**: `#[serde(rename_all = …)]` and `#[strum(serialize_all = …)]` spell the same naming (a consistency test pins it); the accessor stays, the mapping is written once — a hand-written match only where the accessor must compose information
- **One change, one transaction**: the message row and its `ChatEntry` land together; a Turn consumes the accumulation it took only on a normal end, and a redelivery or an older edit is dropped whole
- **Tool calls in a Step run sequentially**, never in parallel
- **`read` does not cache page text**: re-reading a URL re-fetches it; only images reuse a transcription
- **Text that faces the model is written to a standard**: tool docs state only what the JSON schema cannot (purpose, preconditions, result shape); prompts give judgment material rather than compliance clauses — no quotas, no receipts, and no decision the agent cannot see. Code, docs, comments and everything the model reads as a contract (tool descriptions, schema fields, `SYSTEM_PROMPT`, section headings) are English; the built-in persona and chat-type layers exist in Chinese and English, and which one a chat reads — with the output language that follows from it — is the chat's setting, while the operational `agents` and retention `wrap-up` layers are English-only (`docs/topics/prompting.md`)
- **Comments carry only what naming and structure cannot**: a contract, a compatibility requirement, an external-library pitfall, a transaction boundary, test intent. Mechanism and rationale belong in `docs/`, and a comment that restates the code is deleted
- **Artifacts state current facts**: a correction replaces what it corrects — no "previously", no "no longer", no debt narration in code, docs or messages
- **Changes need confirmation**: present the plan before implementing; discussion is not an instruction to change
- **Commits are user-initiated** (the agent reminds when changes accumulate): atomic, Conventional Commits prefix, message written against the final diff
- **External actions** — issues, PRs, comments, tracking entries, decision records — are user-initiated or explicitly consented
- **Privacy**: credentials, personal data and production or intranet details never enter docs or tracking, and information sources are not recorded; an example anywhere — prompt text, docs, tests — is invented, never quoted from a real conversation

</CRITICAL>

Conventions already in force:

- **Errors** are one `AppError` enum (`thiserror`): a local message variant plus typed `#[from]` sources, no codes or families, one neutral arm for a platform's own wire failure (`AppError::Adapter`, so no vendor error type enters the enum), `?` for propagation, `expect` only where the code knows more than the type checker (with the reason stated), `tracing::warn!` for a best-effort step that fails
- **Resource lifetime** is bound to scope with an RAII guard rather than explicit cleanup
- **A named function** carries any call chain or multi-step flow that needs a name, so callers stay free of composition detail
- **Config structs** deny unknown fields, derive defaults and deserialize; keys are kebab-case

## Testing

- Unit tests are few and per-module: list with `cargo test -p hai --lib -- --list`, run with `cargo test`
- Test-only helpers several modules share live in a `#[cfg(test)] mod test_support`: one at the crate root for the whole crate, the rest in that layer's (`agent/`, `domain/`) — fixtures, fakes and assertion helpers; no test lives there
- A fixture for a type a module owns stays in that module behind `#[cfg(test)] mod fixtures`; a test directory is owned by `test_support::TempDir` (created or adopted) and never removed by hand
- Acceptance of core behavior needs a real Telegram bot and PostgreSQL; without them, verification is limited to compilation and tests — say so instead of implying more
- Before delivery, `cargo clippy --all-targets` must be warning-free

## Documentation navigation

Read the matching doc before changing code.

| Task | Doc |
|---|---|
| ChatAgent lifecycle, scheduling, chapter reopen, communication | docs/topics/chat-agent.md |
| Prompt rendering, personality, skills | docs/topics/prompting.md |
| Tools, execution, multimodal, MCP, networking | docs/topics/tools.md |
| Data model, services, value objects, pgvector, schema | docs/topics/domain.md |
| Configuration, error handling, paths, providers | docs/topics/config.md |
| Telegram integration | docs/topics/platform.md |
| Command table, dispatch, menus | docs/topics/commands.md |
| CLI, event-log TUI, rebuild | docs/topics/cli.md |
| Cross-module wiring, event system, assembly | docs/architecture.md |
| Term meanings | docs/CONTEXT.md |

## Documentation maintenance

- **Same turn**: a code change updates the matching doc in the same turn; on conflict the code wins and the doc is corrected
- **Anchors are `file:symbol`**, never line numbers
- **No change log**: a change that forms a behavioral contract is described in the matching `docs/topics/` doc; a purely internal change is not recorded
- **Terms** go into `docs/CONTEXT.md` as soon as they are settled
- **A `README.md` change updates `docs/README.zh-CN.md` in the same turn**: the two files address the same reader, and the Chinese one sits on no other sync list
- **User constraints** (bans, norms, requirements) harden into this file's Code style plus the matching pitfall section of a doc
