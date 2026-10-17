# Data layer

> The data layer: tables and their invariants, service contracts, retrieval, chunking, the schema. Read before changing `domain/` or `crates/hai/migrations/schema.sql`.

## Overview

`domain` is split four ways: `model` (row carriers), `repo` (every SQL statement and every transaction boundary), `service` (business logic, which never touches a sqlx type), and `vo` (value objects). The embedding columns sit **outside** `schema.sql` because their dimension comes from the embedding model role (`[model-roles.embedding] dimension`) — `domain/repo/vector.rs` creates and maintains them at runtime.

## Design

### Tables and what they guarantee

| Table | The invariant that matters |
|---|---|
| memory | `kind` is a user fact, a note or a knowledge entry; invalidation keeps the row and retrieval skips it; `pinned` feeds the chapter-opening block. Every kind carries a chat id and every vector search filters by chat, so a chat-less entry would be unreachable |
| topic, topic_note | The summary is a projection over append-only notes, so rewriting it loses nothing; the time and message ranges are refreshed from the linked messages (`sync_topic_times`), so a topic created without messages keeps the write-moment times until one is linked |
| message | `external_id` is the idempotency key; `text` is the message's own text and `attachment` rows carry its files. `edited_at` is the platform's last edit time and nothing else about an edit is kept — an edit replaces text and attachments in place. There is no unread/seen state: what a Turn has not read yet is the accumulation, not a column |
| attachment | One occurrence of a file in one message: `id` is the occurrence id the model reads, `file_unique_id` the platform's stable handle and `file_id` the download handle; `UNIQUE(message_id, ordinal)` fixes the order and the row follows its message. Re-delivering a platform message keeps its occurrence ids, so a reference the model already holds stays valid. `meta.media_codec` is the media type the platform's own wire declared, which is what an audio or video model is told about; it is absent on rows written before the platform stated one, and the kind's default then stands in |
| perception | A multimodal analysis cache keyed by `(subject_kind, subject, parser, prompt)` — `handle` is `<platform>:<file_unique_id>` and `url` a normalized URL; a row whose `subject_kind` names no known kind is never selected by a lookup, so it costs a re-transcription and nothing else |
| conversation | One row per chat: the accumulated context messages, the chapter scalars, and `pending`, the changes no Turn has consumed yet (a `ChatEntry` per change, ordered by when it happened). It exists to make restore fast, not to be the truth for message text and attachments — an unreadable snapshot field and an unparsable `pending` entry are both errors: rebuilding from a default would consume the batch over an empty context, and an empty backlog would hide the loss |
| event | Observable agent events; the payload is JSONB and there are no dedicated chat/kind columns, so filtering reads into the payload |
| knowledge_document, knowledge_chunk | A document is the lifecycle unit (`source` is the unique key, and the stored content hash decides whether an import skips it) and chunks are the retrieval unit; the chunk sequence is unique per document so ordering survives a rebuild |
| chat_preference | The chat's own setting where one exists (the language it speaks, `docs/topics/prompting.md`); no row means it inherits the configured default, and the blob tolerates fields a newer build added |
| account, chat, identity | Platform identity. A chat is created once and has no update path |
| scheduled_task | One-shot or recurring triggers; the `bot_id` column holds the task's `ChannelKey` (`platform:name`) |

### Service contracts

- **Absence is not an error.** A getter that returns an option means genuine absence when it returns none, and propagates a database failure when it fails — a database error is never converted into "not found".
- **Idempotency lives in the database.** Message external ids, attachment `(message_id, ordinal)`, knowledge document sources, and the two partial perception keys are unique indexes, so a concurrent double write is caught rather than raced.
- **One change, one transaction.** `domain/repo/ingest.rs:PlatformChange` is one platform message already read into the neutral shape (chat, sender, text, attachments, external id, reply-to, change kind, audience, the platform's own meta) which `domain/service/platform.rs:PlatformService::apply_platform_change` turns into a message row and a `ChatEntry` together (`domain/repo/ingest.rs:IngestRepo::apply`), and a due scheduled task appends its entry together with its advanced clock (`domain/repo/scheduled_task.rs:ScheduledTaskRepo::fire`), so a Turn can never consume a change whose content is missing and a fired task can never come back as due. The append and the consumption are relative operations on `pending` (`ConversationRecordRepo::append_entry` and the snapshot `save`, which drops the consumed ids as it writes), which is what lets an ingest and a Turn's commit land without overwriting each other.
- **The bot's own account belongs to one channel.** `domain/service/platform.rs:PlatformService::ensure_bot_account(platform, key)` creates (or returns) the account a channel speaks through, keyed `bot:<channel key>` on its own platform, so several channels never share one identity and the render's `own` marking stays each channel's own. A database written before that key existed holds one shared `(system, 'bot')` row, which the same call adopts in place (`domain/repo/account.rs:AccountRepo::adopt_legacy_bot_account`), keeping the history that references its id.
- **A redelivery is not a change.** The same external id arriving again refreshes the row without a second entry, and an edit whose `edit_date` is not newer than the stored `edited_at` is dropped whole — both read the platform's own signal, never a comparison of content.
- **Memory deduplicates on write**: an exact duplicate returns the existing row instead of erroring, and near neighbors come back as hints so the model can revise instead of re-recording. The lookup's index holds `hashtextextended(content, 0)` rather than the content itself — a btree entry caps at 2704 bytes, which a long entry would fail on — with `content` still compared exactly.
- **Memory and topic mutations carry their own scope**: every write names the chat in its own `WHERE` and returns the row it changed (`domain/repo/memory.rs:MemoryRepo::revise`/`set_status`/`set_pinned`/`delete`, `domain/repo/topic.rs:TopicRepo::close`/`add_note`/`update_fields`/`link_messages`/`delete`), so an id belonging to another chat is unrepresentable rather than something a preceding read has to have caught, and a write that matches no row reports it as not found (`false`/`None` from the repo, the service's own error). A read on the way in remains where it is *input*, not scope: `TopicService::revise` reads the row to compute the embedding for the half the caller did not pass.
- **A write that matches no row fails**: cancelling a scheduled task reports it as not found, instead of succeeding on an update that touched nothing.
- **A failed Turn's writes are the caller's problem.** Transaction boundaries live inside repo methods, so a service composes repo calls but never opens a transaction itself.

### Embedding writes differ by entity on purpose

| Entity | Failure behavior | Why |
|---|---|---|
| memory, topic | fails the call | The vector is part of the row's own write (`MemoryRepo::create`/`revise`, `TopicRepo::open`/`update_fields`), so a row that exists is always one retrieval can find; a failure leaves nothing behind for the model to retry |
| perception | written asynchronously, best-effort | It is a cache; the analysis text is already persisted |
| knowledge chunk | inside the transaction | A chunk without a vector would be invisible to retrieval, so the document write must roll back with it |

### Retrieval

`domain/repo/vector.rs` ranks by L2 distance and skips rows without a vector; under unit-normalized embeddings that ranks the same as cosine. Three entry points keep a search to rows its table can actually narrow: `search_chat_embedding_vec` (memory and topic — the tables carrying `chat_id`), `search_document_embedding_vec` (chunks, the only embedding table hanging off a document) and `search_unscoped_embedding_vec` (no narrowing beyond the vector match). A scope a table cannot serve is therefore unrepresentable rather than a runtime SQL error. The only further predicate is the caller's own, passed as `domain/repo/vector.rs:Filter`: a column literal the table's repo owns plus a value that is bound, so the rule ("invalidated memories are unreachable, a closed topic is not") lives with the table and no wire value is spelled into SQL.

The IVFFlat index is created **only** by `hai db rebuild embeddings` (`domain/repo/vector.rs:rebuild_embedding_indexes`), with the `vector_l2_ops` opclass that matches the `<->` operator. Migrating a database therefore produces vectors with no index, and changing the operator means changing the opclass in the same move.

#### Two stages: fuse the legs, then score the candidates

- **Legs.** The vector leg is the pgvector ranking above, reached through the repo that owns the table (`domain/repo/memory.rs:MemoryRepo::search_by_embedding`); the vector *writes* ride with the row they belong to (`MemoryRepo::create`/`revise` → `domain/repo/vector.rs:vec_to_pgstring`), so a row a reader can see always has the vector retrieval needs. The lexical leg is a `content ILIKE ANY(keywords)` scan ranked by how many keywords hit, and the pattern building lives with the vocabulary (`domain/repo/lexical.rs:patterns`/`:match_predicate`/`:match_rank`) while each repo assembles its statement (`domain/repo/memory.rs:MemoryRepo::search_by_keywords`, `domain/repo/knowledge_chunk.rs:KnowledgeChunkRepo::search_keywords`). The keywords come from `domain/repo/lexical.rs:lexical_terms`: a latin run of three bytes or more is kept whole, and a run carrying CJK is expanded into its adjacent character pairs, because Postgres cannot tokenize CJK and a whole-sentence pattern would match nothing. Automatic recall derives them from the query when the model passes none.
- **Fusion.** `domain/service/fuse.rs:reciprocal_rank_fusion` sums `1/(60 + rank)` over the legs, so neither leg's score scale has to match the other's — memory, topic and knowledge chunks all fuse through it, and only a leg's order survives.
- **Score.** With `[model-roles.rerank]` configured, one request scores the fused candidates (`agentcore/api/rerank.rs:Scorer`, the `rerank` ModelRole) and that score orders the result; `limit` becomes how many survive, and `domain/service/rank.rs:pool` derives the candidate pool from it, capped by `[agent.retrieval] candidates`. Where the request goes is the provider's rerank kind (`agentcore/model_kind.rs:ModelKind`, `agentcore/provider.rs:ProviderKind::endpoint_for`), overridable by the block's `base-url`. The transport and its budgets live in `agentcore::api::rerank::Scorer`; the ranking policy — the pool, the pairing of scores back onto items, and what a failure means (`domain/service/rank.rs:ScorerErrorPolicy`) — lives with the retrieval that applies it. A query carrying no semantic text (keywords or a time window only) has nothing to score against and stays in fusion order. The scores are logged at `debug` for calibration. A cross-encoder's score is comparable only inside one request, so no threshold is applied. A rerank failure is fatal for a tool call and harmless for the automatic chapter recall, which keeps the fusion order (`domain/service/rank.rs:ScorerErrorPolicy`).
- **Per-document trim.** Knowledge hits keep the best chunk of each document (`domain/service/knowledge.rs:best_per_document`) before the cut to `limit`: neighbouring chunks of one document would otherwise fill a limit and hide the other documents that answer the same question.
- **Why not BM25.** Stock Postgres cannot segment CJK — `to_tsvector('simple')` turns a whole run into one token — and a segmenting extension (zhparser, pg_jieba) is something the deployment would have to install. The legs are also ranked by fusion, not by a text score, which is why the lexical leg counts matches instead of weighting them.

### Chunking

`domain/service/chunking/mod.rs` is pure, dependency-free and deterministic — the same input always yields the same chunks, which is what makes a knowledge import idempotent.

- The Markdown heading tree defines sections, and every chunk carries its heading path as a prefix, so a retrieved chunk still says where it came from.
- Code fences, table rows and list items are never cut; a table that has to be split repeats its header. Text splits prefer sentence boundaries.
- Adjacent chunks overlap by about a tenth, except across a structural boundary or a section, where the overlap would duplicate unrelated material.
- The chunker never truncates: a document beyond `domain/service/knowledge.rs:MAX_CHUNKS_PER_DOC` is rejected at import, so nothing is silently dropped.

## Boundaries

- All SQL lives in `repo`, which also owns reading and writing vectors; `service` composes repos and never holds a sqlx type, and no layer builds a transaction outside it.
- Field lists and signatures are not documented here — read the model, repo and service files.
- JSON columns are real JSONB decoded into `serde_json::Value`; an expression index over one depends on that.
- Timestamp columns still convert through `#[sqlx(try_from = "jiff_sqlx::Timestamp")]`, where a failure reaches the log as the column name plus `unprintable error` — accepted because Postgres enforces the column type, so only a value outside `jiff`'s range can fail.
- Conversation state is never expired or deleted.

## Trade-offs and Reasons

- **A snapshot rather than a rebuild on restore**: restoring conversation state from the message table would replay history and lose the accumulated tool chains, so the snapshot is persisted and treated as degradable.
- **Vectors written outside the row's transaction** except for knowledge: it keeps the common paths cheap and avoids holding a transaction across a network-bound embedding call, at the cost of a window where a row has no vector.
- **A destructive embedding rebuild**: clearing every vector and rebuilding the index is simpler than reconciling a partially changed embedding model, and the operation is explicitly invoked.

## Pitfalls / common mistakes

- **Closing a topic is descriptive, not a gate.** A closed topic stays writable, and the summary can still change — `close` only marks the span as over.
- **Topic times come from messages.** Setting them from the write moment silently breaks the range the model reads.
- **`Ok(None)` is a promise.** Converting a database failure into absence hides corruption from every caller up the stack.
- **Enum columns decode through their enums.** `chat.chat_type`, `topic.status`, `memory.kind`/`status`, `message.role`, `attachment.kind` and `perception.subject_kind` decode through the enum's own `sqlx::Decode` (`domain/vo/mod.rs:enum_column`), and `attachment.meta` decodes JSONB into `AttachmentMeta` through its own `sqlx::Type`/`sqlx::Decode`, so a stored value the type cannot take fails the read with a message naming the type and the value (`unknown MemoryStatus value "stale-value"`). Decoding there rather than through `#[sqlx(try_from = …)]` is deliberate: `sqlx` renders a `TryFrom` error through a specialization that collapses to `unprintable error`.
- **A perception key is data, not a document.** The subject is spelled `<platform>:<file_unique_id>` and every writer and reader must spell it the same way. A lookup binds the kind it wants, so a row whose `subject_kind` names no known kind is never returned — a miss, not an error; a read that takes the whole table (`hai db rebuild embeddings`) fails on it with the typed message instead.
- **A column's sentinel is a type, not a value.** `knowledge_document.collection` spells "filed under nothing" as the empty string and decodes into `Collection::Uncategorised` (`domain/vo/collection.rs`), while a collection that has a name carries a `CollectionName` that cannot be empty; `scheduled_task.every_secs` spells "does not repeat" as NULL and decodes into `Option<RepeatInterval>` (`domain/vo/time.rs`), so a stored `0` fails the read with the value in the message. The model's `search_knowledge_base` argument can therefore name a collection or ask for all of them, and never spells the unfiled bucket by accident.
- **An attachment occurrence id is derived, not random.** `domain/vo/id.rs:attachment_id` makes it a function of `(platform, chat_id, external_id, ordinal)`, which is what keeps a model-held `<attachment id>` valid after the same message is stored again. `domain/model/attachment.rs:NewAttachment::new` derives it and takes the platform and the ordinal from its caller; a send whose platform returned no download handle keeps that same derived id as the row's handle, so the occurrence id is the one thing about the row that is always there.
- **Changing an embedding dimension rewrites the whole table** — expect a long rebuild on a large corpus.

## Evolution Direction

`domain/service/mod.rs:Services` receives the embedding implementation as an injected `Arc<dyn EmbeddingService>` (an agentcore trait). Keep that injection boundary when the assembly changes: it is what keeps the data layer free of multimodal dependencies.
