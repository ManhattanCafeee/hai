# Prompt rendering

> System prompt assembly, context XML rendering, personality and skills injection. Required reading when changing `agent/context/`, `agent/role/`, `agent/runtime/prompts.rs` or `agentcore/skills/`. Terms: docs/CONTEXT.md; layering: docs/architecture.md.

## Overview

A Turn's LLM call gets two prompt inputs: a **system prompt**, assembled by `agent/role/default/prompt.rs:SystemPromptBuilder` from the behavioral layers (their override chain resolved once per process) plus the chat's language (resolved once per Turn), and the **context XML** — the Turn's first user message, rebuilt each Turn by the Node engine of `agentcore/render` (`agentcore/render/mod.rs:render_xml`). Within a Turn the system prompt is byte-identical; the context XML carries what is new.

## Design

### System prompt assembly

Sections are appended in this order — `SYSTEM_PROMPT` → `# Character` → `agents` → `# Environment` → `# Private chat` / `# Group chat` → `# Language` → `skills` — and joined by a `\n----------------\n` rule, each section's content trimmed; a blank section is dropped, and a headed one renders as `{heading}\n\n{content}`. `SYSTEM_PROMPT` is the platform and tool-interface contract and is deliberately not configurable; the chat-type section is `group` or `private`, heading-less for a Channel. Every heading is English, because headings are prompt structure rather than voice.

`# Environment` is a built-in block rather than a configurable layer (`agent/runtime/prompts.rs:environment`, English-only) and it is one sentence: `[sandbox] enabled` picks between `You act through a sandbox: a container, not the machine you run on.` and `You act directly on the machine you run on.` It names the environment and stops — the model can look at the tree it got, so what a tool's own caller must know stays in that tool's description.

`soul`, `agents`, `group`, `private` and the separately appended `wrap-up` resolve once at startup (`agent/runtime/prompts.rs:resolve`, one `Layer` variant per layer carrying its override file, `[prompt]` field and built-in text) from `.hai/<KEY>.md` > `[prompt] <key>` > the built-in default, blank values falling through — so an edit takes effect only after a restart; a file that exists but cannot be read fails startup rather than silently falling through (`agent/runtime/prompts.rs:override_file`). Skills share that lifecycle.

### Layer catalogs and the chat's language

The built-in `soul` and `group` layers each exist twice: a Chinese catalog and an English one. The `private` layer ships no built-in text — a private chat renders no chat-type section, and `.hai/PRIVATE.md` or `[prompt] private` is how an operator adds one. `agentcore/language/mod.rs:Language::text_catalog` picks between them — **Chinese reads the Chinese catalog and every other language reads the English one** (`docs/CONTEXT.md` "text catalog"), so adding a language is one enum variant, its two display names and its client code. Text that exists once per catalog is held as `agentcore/language/catalog.rs:PerCatalog<T>` — every literal names both fields, so adding a catalog stops compiling at each site — and text that takes arguments is held as `PerCatalog<fn(..) -> String>`, selected by `PerCatalog::get` and then called. The boundary that holds the chat's language resolves `Language::text_catalog` once and everything below it takes `TextCatalog`: a catalog-level accessor never takes a `Language`, so a text composed of another — a wake's `describe` rendering a command's `instruction` — hands the same catalog down instead of re-deriving it. `TextCatalog` iterates, and a test pins that every catalog is one some language reads, because a catalog no language maps to is text no chat can reach. A language's name travels as the typed `agentcore/language/mod.rs:LanguageName`. A regional variant (`en-gb`, a future `zh-tw`) sits on its base language's catalog the same way, and what it changes is the statement below rather than the prose; it also has no client code of its own — `agentcore/language/mod.rs:Language::client_code` takes the tag's first segment, so a platform matching a client by code serves it its base language's (`docs/topics/platform.md`). The language itself is one aspect of a chat (`agentcore/language/mod.rs:Language`, resolved as `chat preference ?? cfg.agent.language`), chosen once per Turn and carried on `TurnContext`; the chat preference is written by `/lang` (`docs/topics/platform.md`).

The built-in `agents` and `wrap-up` layers carry one text every catalog reads instead (`agent/runtime/prompts.rs:AGENTS` and `WRAPUP`, both fields the same text): operational instructions and the retention pass are machinery, not voice — the reply's language comes from the persona and chat-type layers. The `# Environment` block is the same kind of text, and English-only for the same reason.

The `# Language` section states it as identity — `你的母语是中文。` / `Your native language is Japanese.` — and stops there. A character with a native language still translates, practises a language or quotes someone when asked; writing those cases out as exceptions, or forbidding mixed language, would be spelling out judgments the model already makes.

An operator override (`.hai/<LAYER>.md`, `[prompt] <key>`) is one text and serves every language: whoever writes it has decided what it says.

### Where a rule lives

Layer text is behavioral surface, not documentation, and the same logic lives in exactly one place: a line the layer does not need costs tokens and gives the model something to act on.

- **Tool descriptions** carry only what the JSON schema cannot express — the capability, the format contract and the effect; mutual-exclusion guidance works from one location only and belongs there alone.
- **Behavioral preferences** (tone) live in the configurable `agents` layer, never in a tool description; voice in `soul`; presence — addressee determination — in the chat-type layer; how much an answer covers, in `SYSTEM_PROMPT`.
- Delete-test for a candidate tool-description line: without it, would the tool be used wrongly? Yes → the description; no → `agents`.

**The frame over the voice.** `SYSTEM_PROMPT` and the `agents` layer set what a turn *is* — the character is the same one across turns, everything marked `own` is its own speech, and a turn is noticing what just happened rather than producing output — while `soul` only colours how it speaks. The frame decides whether the model carries itself forward or re-derives a fresh reply each turn; a persona cannot repair a frame that reads every wake as a production task.

**Fit, not sufficiency, sets a turn's size.** What a message leaves unsaid was never owed, so an exchange ending on one line is finished rather than truncated; nothing the person already knows is explained back to them, and where their knowledge ends is theirs to show rather than the agent's to assume. What the agent says is its own read of the matter rather than a survey of it, and sizing an answer by what the other person needs instead makes anything the agent knows a great deal about come back as a report.

**An individual, not a responder.** A message is what the agent has to say rather than an answer to a question, and a wake that carries no question — scheduled, self-scheduled through `schedule`, or a heat dispatch — is the same kind of turn as an addressed message. Wording that makes every turn look like a query turns the agent into a responder, whatever the persona says.

Prompts give judgment material, not compliance clauses — no quotas, no receipts — and the mechanism never makes a decision the agent cannot see.

### Context XML rendering

`agent/context/builder.rs:build_prompt` renders one `<context>` root through `agent/context/sections/context.rs:render_main_context`; children come in this order, and an empty section is dropped.

```mermaid
flowchart LR
  context --> situation & environment & chat & accounts & current & recall & perceptions & conversation
```

The first-turn children are the chapter's opening state, rendered once: the bot's identity, the chat and its participants, the agent's own marks (open topics, stale past `topic_idle_hours` and carrying `need-close`, plus pinned memories) and the system's recall from this Turn's window text, excluding what `current` already carries (`agent/context/load/recall.rs:chapter_recall`). The agent mutates some of that mid-chapter — the `topic` and `memory` tools close a topic or pin a memory — and those changes reach the model through the tool results it already holds, not by re-rendering the block; the opening state refreshes with the next chapter. Every Turn renders the situation; the rest of the render adds the current time.

`<accounts>` is ordered by account id (`agent/context/load/chat.rs:collect_accounts`): the opening state of the same chat must render the same prompt, and the order a hash map happened to produce would not.

**Accumulation rule.** Rendering appends to `context_messages`, so every item must appear exactly once there; `agent/context/render_context.rs:RenderContextData`'s `first_turn` is the switch.

**First turn means chapter initial state** — `agent/runtime/chat/conversation.rs:Conversation::is_first_render()` is `turn_count == 0`, not "a fresh agent" or "no context messages": a chapter opened by a wrap-up summary already holds that message and is still its first render, and a failed Turn never advances `turn_count` (docs/topics/chat-agent.md).

**Situation-only branch.** With nothing gathered on a non-first turn, `build_prompt` returns a `<context>` whose only child is the situation section (`agent/context/sections/context.rs:render_situation_only`): a scheduled task or a command has no message of its own, so this is the only place its instruction can travel. An empty prompt would leave the model with neither instruction nor input.

### Personality (soul)

The section is `You are {name}.\n{soul}` under the `# Character` heading, `config/schema.rs:PersonalityConfig.name` feeding the name. `soul` is the identity and voice body — one piece of free-form prose, no structured parsing. Prose rather than a rule list, because a prompt's shape leaks into the reply's shape: a bullet list asking for a chat register reads as two conflicting instructions.

### The wrap-up layer

`.hai/WRAPUP.md` > `[prompt] wrap-up` > `agent/runtime/prompts.rs:WRAPUP` is its own layer, one English text appended as a user message in the wrap-up pass rather than rendered into the system prompt; the pass reuses the Default system prompt byte-identically (`agent/role/mod.rs:AgentRole::spec`), so the provider cache survives it. A structured extraction task, it stays detailed where the conversational layers stay short. The layer keeps the pass's two channels apart: the retention summary is the pass's text output (the next chapter's opening), while `memory` files the stretch's standalone facts — the summary is never one of them.

### Skills injection

`agentcore/skills/manager.rs:SkillManager::discovery_prompt` emits a `## Skills` section when at least one skill is discoverable: one entry per skill plus the instruction to load a matching one with `read skill://<name>`. `discoverable_skills` excludes skills whose front matter sets `disable-model-invocation`, and `read`'s description mentions `skill://` only while skills exist.

## Boundaries

- Node shapes and field lists are code facts; read `agent/context/sections/`.

## Trade-offs and Reasons

- **WrapUp reuses the Default prompt and tool surface** rather than a minimal executor: "cannot speak, cannot skip" is enforced at runtime instead of through a narrowed schema (`RoleSpec::rejects`).
- **Chapter-opening injection rather than per-Turn retrieval**: the model gets an index of what it knows and calls `recall` itself afterwards.

## Pitfalls / common mistakes

- **Register instructions decay over a long chapter**: the prompt is fixed per Turn while the chapter grows, and nothing re-injects a style reminder mid-chapter.
- **Two rules that cancel cost more than either is worth**: the model reconciles them itself, obeying each rule nominally while producing the wrong shape.
- **An example is copied, not just read**: a prompt sample, a doc snippet or a test fixture quoting a real conversation ships that conversation to every deployment — examples are invented.
- **Repetition is a frame problem, not a thin persona**: a frame that reads each wake as fresh output makes the model re-derive the same reply and restate it, and its own prior turns in the context reinforce the pattern; continuity in the frame is the lever, not another persona line.
- **`agentcore/render/elements.rs:Node::is_empty` means no children *and* no attributes**, so an attribute-only node such as `<chat …/>` counts as content and is kept.
- **The divider is the batch, not a stored flag**: `RenderContextData::new_message_ids` holds the message ids this Turn's accumulation changed (`agent/context/sections/message.rs:conversation_element`), and the `separator` is rendered at the first of them — keep the single-divider convention. A message renders `edited_at="…"` once the platform has edited it, and a reference whose sender chose an excerpt renders that excerpt as a `<quote>` child; neither is a message of its own.

## Evolution Direction

- **Per-tool text carrier**: a tool description that outgrows roughly ten lines, or that needs a table or a worked example, should move into its own prompt carrier — none exists today, because current tool syntax does not need one.
