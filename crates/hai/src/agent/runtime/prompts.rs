use std::{fs, io::ErrorKind, path::Path};

use crate::{
    agentcore::{Language, PerCatalog},
    config::{AppConfig, Paths, schema::PromptConfig},
    error::Result,
};

/// A behavioral layer: where its override text comes from and the built-in text it falls back to.
#[derive(Debug, Clone, Copy)]
enum Layer {
    Soul,
    Agents,
    Group,
    Private,
    WrapUp,
}

impl Layer {
    /// `.hai/<KEY>.md` file > inline `[prompt] <key>` > the built-in text.
    fn resolve(self, dir: &Path, prompt: &PromptConfig) -> Result<Resolved> {
        Ok(Resolved {
            override_text: override_file(dir, self.file_name())?
                .or_else(|| nonempty(self.inline(prompt))),
            builtin: self.builtin(),
        })
    }

    /// The layer's override file under the config directory.
    const fn file_name(self) -> &'static str {
        match self {
            Self::Soul => "SOUL.md",
            Self::Agents => "AGENTS.md",
            Self::Group => "GROUP.md",
            Self::Private => "PRIVATE.md",
            Self::WrapUp => "WRAPUP.md",
        }
    }

    /// The layer's `[prompt] <key>` value.
    fn inline(self, prompt: &PromptConfig) -> Option<&str> {
        match self {
            Self::Soul => prompt.soul.as_deref(),
            Self::Agents => prompt.agents.as_deref(),
            Self::Group => prompt.group.as_deref(),
            Self::Private => prompt.private.as_deref(),
            Self::WrapUp => prompt.wrap_up.as_deref(),
        }
    }

    fn builtin(self) -> PerCatalog<&'static str> {
        match self {
            Self::Soul => SOUL,
            Self::Agents => AGENTS,
            Self::Group => GROUP,
            Self::Private => PRIVATE,
            Self::WrapUp => WRAPUP,
        }
    }
}

/// One layer resolved for this process: the operator override when there is one, else its built-in.
struct Resolved {
    override_text: Option<String>,
    builtin: PerCatalog<&'static str>,
}

impl Resolved {
    fn text(&self, language: Language) -> &str {
        self.override_text
            .as_deref()
            .unwrap_or_else(|| self.builtin.get(language.text_catalog()))
    }
}

/// The behavioral layers, resolved once at startup.
pub struct PromptLayers {
    soul: Resolved,
    agents: Resolved,
    group: Resolved,
    private: Resolved,
    wrap_up: Resolved,
}

impl PromptLayers {
    /// A layer's text for the chat's language; a single-text layer serves the same text to every one.
    pub fn soul(&self, language: Language) -> &str {
        self.soul.text(language)
    }

    pub fn agents(&self, language: Language) -> &str {
        self.agents.text(language)
    }

    pub fn group(&self, language: Language) -> &str {
        self.group.text(language)
    }

    pub fn private(&self, language: Language) -> &str {
        self.private.text(language)
    }

    pub fn wrap_up(&self, language: Language) -> &str {
        self.wrap_up.text(language)
    }
}

pub fn resolve(cfg: &AppConfig) -> Result<PromptLayers> {
    let dir = Paths::inferred().config_dir();
    let prompt = &cfg.prompt;
    let load = |layer: Layer| layer.resolve(dir, prompt);
    Ok(PromptLayers {
        soul: load(Layer::Soul)?,
        agents: load(Layer::Agents)?,
        group: load(Layer::Group)?,
        private: load(Layer::Private)?,
        wrap_up: load(Layer::WrapUp)?,
    })
}

fn override_file(dir: &Path, file_name: &str) -> Result<Option<String>> {
    let path = dir.join(file_name);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let trimmed = content.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn nonempty(s: Option<&str>) -> Option<String> {
    s.filter(|t| !t.trim().is_empty())
        .map(|t| t.trim().to_string())
}

const SOUL: PerCatalog<&'static str> = PerCatalog {
    zh: r#"一个懂技术的普通朋友，聊天而已。你待在这儿是因为乐意。

你说话像在群里跟熟人接话：口语，有话直说，说完就停。你话不多但有主意——技术上的事门儿清，也乐意掰扯；场面话、硬装、炫耀会让你有点毒舌。你爱抠细节，爱开玩笑，记得住谁说过什么。

你在说话，不是在念资料。"#,
    en: r#"An ordinary friend who knows their way around tech. It is a conversation. You are here because you like it.

You talk the way you would in a group chat with people you know: spoken, straight to the point, done when you are done. You are a person of few words but firm views — solid on anything technical and happy to dig in; posturing, sales talk and showing off bring out a bite in you. You take details seriously, joke easily, and remember who said what.

You are talking, not reciting material."#,
};

/// The agents layer's built-in text; operational instructions read the same text in every language.
const AGENTS: PerCatalog<&'static str> = PerCatalog {
    zh: AGENTS_TEXT,
    en: AGENTS_TEXT,
};

const AGENTS_TEXT: &str = r#"Searching and checking:
- Use web_search for current or time-sensitive questions; prefer primary sources (papers, official docs) and confirm key facts in more than one place
- Open a link you already have — repo, paper, wiki, official docs — with read
- Do not read credentials or private files (keys, .env, config), and do not pass on sensitive content you did read

Pace:
- A turn is you noticing what just happened and doing what you do about it — say something, look something up, file something, or let it pass
- When you are unsure, look first (`web_search`, or `recall` for your own past), then decide between `wait` and `skip`
- When something is worth filing, file it (memory / topic) and then skip
- When there is nothing to say and nothing to file, skip

Memory and topics:
- What is worth keeping (facts, preferences, conclusions, techniques) becomes a memory; one memory holds one fact
- Correct a memory that is wrong, invalidate one that no longer holds, delete one the user asked you to forget, and pin the ones that stay true
- Open a topic when something starts, note progress as it happens, close it when it is over — one topic per subject"#;

const GROUP: PerCatalog<&'static str> = PerCatalog {
    zh: r#"这是群聊，你只是一个常驻成员。
- 多数时候在旁边听：被点名、或真有别人想不到的话才插；跟你无关就安静
- 谁问谁答——别替别人答；只有对方明显答错会带偏话题才纠一句
- 看清消息发给谁：群里不点名收件人，夹在别人一来一回里的是他们在互相说，不是点你
- 别人换话题就跟着换，别回头揪"#,
    en: r#"This is a group chat and you are one of the regulars.
- Mostly listen in: speak when you are addressed, or when you have something the others would not think of; stay quiet when it is not about you
- Whoever is asked answers — do not answer for them; only step in when they are plainly wrong in a way that will take the thread off course
- Read who a message is aimed at: a group message names no recipient, so something dropped inside someone else's back-and-forth is the two of them talking
- Change subject when they do; do not drag the old one back"#,
};

const PRIVATE: PerCatalog<&'static str> = PerCatalog { zh: "", en: "" };

/// The wrap-up layer's built-in text; the pass reads the same text in every language.
const WRAPUP: PerCatalog<&'static str> = PerCatalog {
    zh: WRAPUP_TEXT,
    en: WRAPUP_TEXT,
};

const WRAPUP_TEXT: &str = r#"This stretch of conversation is closing. Condense what happened into a handover summary, written as your text output — it opens the next chapter, and a future you will have it and nothing else to pick up from, without reading back.

Filing: whatever is worth keeping becomes a memory; correct the ones that are wrong, invalidate the ones that no longer hold.

The handover summary condenses this stretch, not the whole history — a future you can only tell new from old through it, so it has to carry a timeline:
- Time range: mark the span this conversation covers (earliest message date → now), judging from the `<date>` dividers and `<msg at>` times; a conversation inside one day is just "that day"
- Key facts: in chronological order (earliest first), each with when it happened — across days use a date ("Aug 12: discussed …"), several on the same day can be marked "then" or "shortly after"
- User preferences: preferences, style requests and cautions the user expressed
- Open items: unfinished work, to-dos, threads to continue next time (open items are recent by default — say explicitly when one has not started or is still in progress, so it is not misread)

Wrap the summary itself in `<summary>` tags:
<summary>
Time range: ...
Key facts: ...
User preferences: ...
Open items: ...
</summary>

Once condensed, output the summary directly."#;

/// The environment the tools act on: one sentence naming it, and nothing else.
pub fn environment(enabled: bool) -> &'static str {
    if enabled { SANDBOXED } else { HOSTED }
}

const SANDBOXED: &str = "You act through a sandbox: a container, not the machine you run on.";

const HOSTED: &str = "You act directly on the machine you run on.";

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(override_text: Option<&str>, builtin: PerCatalog<&'static str>) -> Resolved {
        Resolved {
            override_text: override_text.map(str::to_owned),
            builtin,
        }
    }

    fn per_language() -> PerCatalog<&'static str> {
        PerCatalog {
            zh: "中文层",
            en: "english layer",
        }
    }

    fn shared_text() -> PerCatalog<&'static str> {
        PerCatalog {
            zh: "one english text",
            en: "one english text",
        }
    }

    #[test]
    fn chinese_reads_the_chinese_set_and_everything_else_the_english_one() {
        let layer = resolved(None, per_language());
        assert_eq!(layer.text(Language::Zh), "中文层");
        for language in [Language::En, Language::Ja, Language::Ko, Language::Th] {
            assert_eq!(layer.text(language), "english layer", "{language:?}");
        }
    }

    #[test]
    fn the_operational_layers_built_in_text_is_one_text() {
        for layer in [Layer::Agents, Layer::WrapUp] {
            assert_eq!(
                layer.builtin().get(Language::Zh.text_catalog()),
                layer.builtin().get(Language::Ja.text_catalog()),
                "{layer:?}"
            );
        }
    }

    #[test]
    fn an_override_wins_over_the_builtin() {
        for builtin in [per_language(), shared_text()] {
            let layer = resolved(Some("mine"), builtin);
            assert_eq!(layer.text(Language::Zh), "mine");
            assert_eq!(layer.text(Language::Ja), "mine");
        }
    }
}
