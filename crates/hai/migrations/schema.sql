-- Embedding vector columns are not here: domain/repo/vector.rs adds them at runtime, since their
-- dimension comes from configuration.

CREATE TABLE IF NOT EXISTS account (
    id              BIGSERIAL PRIMARY KEY,
    identity_id     UUID,
    platform        TEXT NOT NULL,
    external_id     TEXT NOT NULL,
    meta            JSONB,
    last_active_at  TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_account_identity_id ON account (identity_id);
CREATE UNIQUE INDEX IF NOT EXISTS uq_account_platform_external ON account (platform, external_id);

CREATE TABLE IF NOT EXISTS chat (
    id          BIGSERIAL PRIMARY KEY,
    platform    TEXT NOT NULL,
    external_id TEXT NOT NULL,
    chat_type   TEXT NOT NULL,
    name        TEXT,
    meta        JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_chat_platform_external ON chat (platform, external_id);

CREATE TABLE IF NOT EXISTS conversation (
    chat_id      BIGINT PRIMARY KEY,
    messages     JSONB NOT NULL,
    context_meta JSONB NOT NULL,
    pending      JSONB NOT NULL DEFAULT '[]'::jsonb,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS event (
    seq        BIGSERIAL PRIMARY KEY,
    payload    JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Payload filters; an expression index is only used where it matches the query expression exactly.
CREATE INDEX IF NOT EXISTS idx_event_payload_chat ON event (((payload->>'chat_id')::bigint));
CREATE INDEX IF NOT EXISTS idx_event_payload_event ON event (((payload->'payload'->>'event')));

CREATE TABLE IF NOT EXISTS identity (
    id         UUID PRIMARY KEY,
    name       TEXT,
    meta       JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS knowledge_chunk (
    id          UUID PRIMARY KEY,
    document_id UUID NOT NULL,
    seq         INT NOT NULL,
    content     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (document_id, seq)
);

CREATE TABLE IF NOT EXISTS knowledge_document (
    id         UUID PRIMARY KEY,
    title      TEXT NOT NULL,
    collection TEXT NOT NULL,
    source     TEXT NOT NULL,
    content    TEXT NOT NULL,
    meta       JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Import idempotency key.
CREATE UNIQUE INDEX IF NOT EXISTS uq_knowledge_document_source ON knowledge_document (source);
CREATE INDEX IF NOT EXISTS idx_knowledge_document_collection ON knowledge_document (collection);

CREATE TABLE IF NOT EXISTS memory (
    id                 UUID PRIMARY KEY,
    account_id         BIGINT,
    chat_id            BIGINT NOT NULL,
    kind               TEXT NOT NULL,
    content            TEXT NOT NULL,
    source_message_ids BIGINT[] NOT NULL DEFAULT '{}',
    status             TEXT NOT NULL,
    pinned             BOOLEAN NOT NULL DEFAULT FALSE,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_memory_chat_id ON memory (chat_id);
-- Index the content hash, not the content: a btree entry caps at 2704 bytes. The lookup compares
-- `content` exactly, so a hash collision only costs one extra row.
CREATE INDEX IF NOT EXISTS idx_memory_dedup ON memory (kind, chat_id, hashtextextended(content, 0));
CREATE INDEX IF NOT EXISTS idx_memory_pinned ON memory (chat_id) WHERE pinned;

CREATE TABLE IF NOT EXISTS message (
    id                 BIGSERIAL PRIMARY KEY,
    chat_id            BIGINT NOT NULL,
    account_id         BIGINT,
    role               TEXT NOT NULL,
    text               TEXT NOT NULL DEFAULT '',
    topic_id           UUID,
    reply_to_id        BIGINT,
    external_id        TEXT,
    meta               JSONB,
    sent_at            TIMESTAMPTZ,
    edited_at          TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_message_chat_id ON message (chat_id);
-- Idempotency key; external_id is nullable, so the index is partial and NULL rows stay free.
CREATE UNIQUE INDEX IF NOT EXISTS uq_message_chat_external ON message (chat_id, external_id) WHERE external_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_message_topic_id ON message (topic_id);

-- One occurrence of a file in one message; `file_unique_id` is the platform's stable handle and
-- `file_id` the download handle. `kind` is the `<attachment type="…">` value.
CREATE TABLE IF NOT EXISTS attachment (
    id             UUID PRIMARY KEY,
    message_id     BIGINT NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    ordinal        INT NOT NULL,
    platform       TEXT NOT NULL,
    file_unique_id TEXT,
    file_id        TEXT NOT NULL,
    kind           TEXT NOT NULL,
    caption        TEXT,
    meta           JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (message_id, ordinal)
);
CREATE INDEX IF NOT EXISTS idx_attachment_handle ON attachment (platform, file_unique_id);

-- A multimodal analysis cache keyed by subject (`handle` = <platform>:<file_unique_id>, `url`),
-- parser and focus (the column is `prompt`).
CREATE TABLE IF NOT EXISTS perception (
    id           UUID PRIMARY KEY,
    subject_kind TEXT NOT NULL,
    subject      TEXT NOT NULL,
    parser       TEXT NOT NULL,
    prompt       TEXT,
    content      TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Idempotency key (subject_kind, subject, parser, prompt): prompt is nullable, hence two partial indexes.
CREATE UNIQUE INDEX IF NOT EXISTS uq_perception_subject_prompt
    ON perception (subject_kind, subject, parser, prompt) WHERE prompt IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uq_perception_subject
    ON perception (subject_kind, subject, parser) WHERE prompt IS NULL;

CREATE TABLE IF NOT EXISTS topic (
    id               UUID PRIMARY KEY,
    chat_id          BIGINT NOT NULL,
    title            TEXT,
    summary          TEXT,
    status           TEXT NOT NULL,
    first_message_id BIGINT,
    last_message_id  BIGINT,
    started_at       TIMESTAMPTZ NOT NULL,
    last_active_at   TIMESTAMPTZ NOT NULL,
    closed_at        TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS topic_note (
    id          BIGSERIAL PRIMARY KEY,
    topic_id    UUID NOT NULL,
    text        TEXT NOT NULL,
    message_ids BIGINT[] NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_topic_note_topic ON topic_note (topic_id, id);
CREATE INDEX IF NOT EXISTS idx_topic_chat_id ON topic (chat_id);
CREATE INDEX IF NOT EXISTS idx_topic_chat_status_active
    ON topic (chat_id, status, last_active_at DESC);

CREATE TABLE IF NOT EXISTS scheduled_task (
    id          UUID PRIMARY KEY,
    bot_id      TEXT NOT NULL,
    chat_id     BIGINT NOT NULL,
    description TEXT NOT NULL,
    fire_at     TIMESTAMPTZ NOT NULL,
    every_secs  BIGINT,
    is_active   BOOLEAN NOT NULL DEFAULT TRUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_scheduled_task_due ON scheduled_task (is_active, fire_at);
CREATE INDEX IF NOT EXISTS idx_scheduled_task_bot_chat ON scheduled_task (bot_id, chat_id);

CREATE TABLE IF NOT EXISTS chat_preference (
    chat_id    BIGINT PRIMARY KEY,
    prefs      JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
