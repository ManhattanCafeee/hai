use std::sync::Arc;

use derive_more::Deref;
use strum::IntoStaticStr;

use crate::{
    agentcore::{Language, PerCatalog, TextCatalog},
    domain::{
        model::ChatType,
        vo::{AgentCommand, ChatEntry, EntryAudience, EntryKind, ScheduledTaskId, WakeReasonKind},
    },
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct TaskPayload {
    pub task_id: Option<ScheduledTaskId>,
    pub description: String,
}

impl TaskPayload {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            task_id: None,
            description: description.into(),
        }
    }

    pub fn with_id_opt(mut self, task_id: Option<ScheduledTaskId>) -> Self {
        self.task_id = task_id;
        self
    }
}

impl AgentCommand {
    /// The instruction text handed to the agent, read in `catalog`; `describe` injects it into the
    /// context. The command is a domain value; its wording is the agent's.
    pub fn instruction(&self, catalog: TextCatalog) -> String {
        match self {
            Self::OrganizeMemory => ORGANIZE_MEMORY.get(catalog).into(),
            Self::Explain(target) => (EXPLAIN.get(catalog))(target.as_deref()),
            Self::Digest(days) => (DIGEST.get(catalog))(*days),
        }
    }
}

const ORGANIZE_MEMORY: PerCatalog<&'static str> = PerCatalog {
    zh: "执行记忆/主题整理，包括处理不符合规范的记忆或主题，删除重建",
    en: "Tidy up memory and topics — including repairing or rebuilding entries that no longer hold up",
};

const DIGEST: PerCatalog<fn(u32) -> String> = PerCatalog {
    zh: digest_zh,
    en: digest_en,
};

const EXPLAIN_STYLE: PerCatalog<&'static str> = PerCatalog {
    zh: "说清是什么、怎么用；版本号、参数、命令这类具体事实先核实，不确定就说不确定",
    en: "say what it is and how it is used; check concrete facts like version numbers, parameters and commands first, and say so when you are unsure",
};

const EXPLAIN: PerCatalog<fn(Option<&str>) -> String> = PerCatalog {
    zh: explain_zh,
    en: explain_en,
};

fn digest_zh(days: u32) -> String {
    format!(
        "客观总结最近 {days} 天聊过的、值得注意的内容：\
         先主动用 recall 检索最近 {days} 天时间范围内的话题\
         （必要时配合语义关键词），再回看相关消息原文核实细节，不凭印象写。\
         输出格式：\
         ① 首行：「最近 {days} 天（几月几日–几月几日）」+ 一句总体情况；\
         ② 主体：按话题分组逐条列要点，每条必须写清三件事——\
         时间（几月几日，能到具体时刻更好）、\
         内容（关键结论、决定、数字或原话要点，不用空泛概括代替）、\
         为什么值得注意（进展、变化、待办、悬而未决）；\
         ③ 结尾：有还没下文的事项时，单列「未决事项」，写清谁在等谁、等什么；\
         没有就不写。语言直白完整：不省略主语，\
         不用「那事」「那个」这类读者没见过的指代，不写需要猜的省略句。"
    )
}

fn digest_en(days: u32) -> String {
    format!(
        "Give an objective summary of what was worth noting in the last {days} days: \
         start by using recall over the last {days} days to find the topics in that range \
         (semantic keywords when they help), then reopen the underlying messages to check \
         the details — do not write from impression. \
         Format: \
         (1) first line: \"the last {days} days (Mon D – Mon D)\" plus one sentence on the \
         overall state; \
         (2) body: group by topic, one bullet per point, each stating three things — \
         when (the day, and the time when it matters), \
         what (conclusions, decisions, numbers or the gist of what was said, never a vague \
         paraphrase), \
         and why it is worth noting (progress, change, a to-do, something unresolved); \
         (3) closing: when something is still open, a separate \"Open items\" list saying \
         who is waiting on whom and for what; leave it out when nothing is. \
         Write plainly and completely: keep the subject in, never \"that thing\" or \"the \
         other one\" the reader has not seen, and no elliptical sentence that has to be \
         guessed."
    )
}

fn explain_zh(target: Option<&str>) -> String {
    match target {
        Some(target) => format!("解释「{target}」：{}", EXPLAIN_STYLE.zh),
        None => format!(
            "解释上文刚提到、还没解释过的那个对象——最近一条消息里带着对象就用它，\
             否则从最近几条消息里挑被提到且尚未解释的那个（对方提的、你自己刚提的都算）；\
             确实挑不出来就一句话问清指哪个。{}",
            EXPLAIN_STYLE.zh
        ),
    }
}

fn explain_en(target: Option<&str>) -> String {
    match target {
        Some(target) => format!("Explain \"{target}\": {}", EXPLAIN_STYLE.en),
        None => format!(
            "Explain the object mentioned just now that you have not explained yet — use it \
             if the last message carries one, otherwise take from the last few messages \
             whichever was mentioned and is still unexplained (whether they raised it or you \
             did); if nothing qualifies, ask in one line which one is meant. {}",
            EXPLAIN_STYLE.en
        ),
    }
}

/// Why the agent was woken, derived from the accumulated change it is about to read.
///
/// A wake steers no behavior beyond `Command`, which carries an explicit user instruction: the
/// agent decides how to respond.
#[derive(Debug, Clone, PartialEq, Eq, Hash, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum WakeReason {
    /// A new message the attention system noticed.
    Observe,
    /// A private message addressed to the agent.
    Direct,
    /// The agent was mentioned in a group.
    Mention,
    Scheduled(TaskPayload),
    Command(AgentCommand),
}

impl WakeReason {
    /// The wake's wire form: what the event log stores, what the situation section renders, and what
    /// every other persisted value in this crate spells in snake_case.
    pub fn label(&self) -> &'static str {
        self.into()
    }

    /// The kind this wake belongs to, dropping the payload a Turn record does not store.
    pub fn kind(&self) -> WakeReasonKind {
        match self {
            Self::Observe => WakeReasonKind::Observe,
            Self::Direct => WakeReasonKind::Direct,
            Self::Mention => WakeReasonKind::Mention,
            Self::Scheduled(_) => WakeReasonKind::Scheduled,
            Self::Command(_) => WakeReasonKind::Command,
        }
    }

    /// The situational text rendered into the prompt for this wake. `Command` renders
    /// its instruction instead of a situation.
    pub fn describe(&self, language: Language) -> String {
        let catalog = language.text_catalog();
        match self {
            Self::Observe => OBSERVE.get(catalog).into(),
            Self::Direct => DIRECT.get(catalog).into(),
            Self::Mention => MENTION.get(catalog).into(),
            Self::Scheduled(payload) => (SCHEDULED.get(catalog))(payload),
            Self::Command(command) => command.instruction(catalog),
        }
    }

    /// The reason one accumulated change stands for: who it was aimed at decides the reading, and a
    /// command or a scheduled task carries its own instruction.
    pub fn of_entry(entry: &ChatEntry, chat_type: ChatType) -> Self {
        match &entry.kind {
            EntryKind::MessageCreated { .. } | EntryKind::MessageEdited { .. } => {
                match (chat_type, entry.audience) {
                    (ChatType::Private, _) => Self::Direct,
                    (_, EntryAudience::Agent) => Self::Mention,
                    (_, EntryAudience::Bystanders) => Self::Observe,
                }
            }
            EntryKind::Command(command) => Self::Command(command.clone()),
            EntryKind::Scheduled {
                task_id,
                description,
            } => Self::Scheduled(TaskPayload::new(description.clone()).with_id_opt(*task_id)),
        }
    }
}

const OBSERVE: PerCatalog<&'static str> = PerCatalog {
    zh: "群里有人在说话。",
    en: "People are talking in the group.",
};

const DIRECT: PerCatalog<&'static str> = PerCatalog {
    zh: "有人发来私信。",
    en: "Someone sent you a private message.",
};

const MENTION: PerCatalog<&'static str> = PerCatalog {
    zh: "有人在群里提到你。",
    en: "Someone mentioned you in a group.",
};

const SCHEDULED: PerCatalog<fn(&TaskPayload) -> String> = PerCatalog {
    zh: scheduled_zh,
    en: scheduled_en,
};

fn scheduled_zh(payload: &TaskPayload) -> String {
    match payload.task_id {
        Some(id) => format!("定时任务 [TaskID:{}]：{}。", id, payload.description),
        None => format!("定时任务：{}。", payload.description),
    }
}

fn scheduled_en(payload: &TaskPayload) -> String {
    match payload.task_id {
        Some(id) => format!("Scheduled task [TaskID:{}]: {}.", id, payload.description),
        None => format!("Scheduled task: {}.", payload.description),
    }
}

#[derive(Debug, Clone, Deref, PartialEq)]
pub struct WakeEvent(Arc<WakeReason>);

impl WakeEvent {
    pub fn new(reason: WakeReason) -> Self {
        Self(Arc::new(reason))
    }
}

#[derive(Debug, Clone, Default)]
pub struct WakeEvents(Vec<WakeEvent>);

impl WakeEvents {
    pub fn new(events: Vec<WakeEvent>) -> Self {
        Self(events)
    }

    /// The reason a turn's row keeps: the strongest one the batch carried, not the first to arrive.
    pub fn strongest(&self) -> Option<WakeReasonKind> {
        self.0
            .iter()
            .map(|event| event.kind())
            .max_by_key(|kind| kind.strength())
    }

    pub fn coalesce(&self, language: Language) -> Vec<EventGroup> {
        let mut groups = Vec::new();
        let mut i = 0;
        while i < self.0.len() {
            let label = self.0[i].label();
            let describe = self.0[i].describe(language);
            let mut count = 1;
            while i + count < self.0.len() && self.0[i + count] == self.0[i] {
                count += 1;
            }
            groups.push(EventGroup {
                label,
                describe,
                count,
            });
            i += count;
        }
        groups
    }
}

#[derive(Debug)]
pub struct EventGroup {
    pub label: &'static str,
    pub describe: String,
    pub count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> Vec<AgentCommand> {
        vec![
            AgentCommand::OrganizeMemory,
            AgentCommand::Explain(None),
            AgentCommand::Explain(Some("lifetimes".into())),
            AgentCommand::Digest(3),
        ]
    }

    /// The label is the value the event log stores and the situation section renders, so it is the
    /// crate's usual snake_case wire form — pinned here because a missing attribute would silently
    /// spell it `Observe` again.
    #[test]
    fn a_wake_label_is_its_snake_case_wire_form() {
        assert_eq!(WakeReason::Observe.label(), "observe");
        assert_eq!(WakeReason::Direct.label(), "direct");
        assert_eq!(WakeReason::Mention.label(), "mention");
        assert_eq!(
            WakeReason::Scheduled(TaskPayload::new("take the medication")).label(),
            "scheduled"
        );
        assert_eq!(
            WakeReason::Command(AgentCommand::Digest(3)).label(),
            "command"
        );
    }

    /// A turn's row keeps the strongest reason its batch carried: an arrival that was only
    /// overheard must not label a turn the agent was mentioned in.
    #[test]
    fn a_batch_keeps_its_strongest_reason() {
        let batch = |reasons: Vec<WakeReason>| {
            WakeEvents::new(reasons.into_iter().map(WakeEvent::new).collect())
        };

        assert_eq!(
            batch(vec![
                WakeReason::Observe,
                WakeReason::Mention,
                WakeReason::Observe
            ])
            .strongest(),
            Some(WakeReasonKind::Mention)
        );
        assert_eq!(
            batch(vec![
                WakeReason::Observe,
                WakeReason::Scheduled(TaskPayload::new("take the medication"))
            ])
            .strongest(),
            Some(WakeReasonKind::Scheduled)
        );
        assert_eq!(
            batch(vec![WakeReason::Observe]).strongest(),
            Some(WakeReasonKind::Observe)
        );
        assert_eq!(batch(vec![]).strongest(), None);
    }

    /// Both texts are translated, and a language that reads the English catalog gets the English
    /// one — a command wake never pins itself to a single catalog.
    #[test]
    fn a_command_wake_follows_the_catalog_its_language_reads() {
        for command in commands() {
            let chinese = command.instruction(TextCatalog::Zh);
            let english = command.instruction(TextCatalog::En);
            assert_ne!(chinese, english, "{command:?}");

            let wake = WakeReason::Command(command);
            assert_eq!(wake.describe(Language::Zh), chinese);
            assert_eq!(wake.describe(Language::Ja), english);
        }
    }

    /// The reason a turn reads follows who the change was aimed at, and a private chat is always
    /// addressed to the agent.
    #[test]
    fn an_entry_reads_as_the_reason_it_stands_for() {
        use crate::domain::vo::{EntryAudience, EntryKind, MessageId};

        let at = jiff::Timestamp::from_second(1_700_000_000).expect("timestamp");
        let created = |audience| {
            ChatEntry::new(
                audience,
                EntryKind::MessageCreated {
                    message_id: MessageId(1),
                },
                at,
            )
        };

        assert_eq!(
            WakeReason::of_entry(&created(EntryAudience::Bystanders), ChatType::Private),
            WakeReason::Direct
        );
        assert_eq!(
            WakeReason::of_entry(&created(EntryAudience::Agent), ChatType::Group),
            WakeReason::Mention
        );
        assert_eq!(
            WakeReason::of_entry(&created(EntryAudience::Bystanders), ChatType::Group),
            WakeReason::Observe
        );

        let command = ChatEntry::new(
            EntryAudience::Agent,
            EntryKind::Command(AgentCommand::Digest(3)),
            at,
        );
        assert_eq!(
            WakeReason::of_entry(&command, ChatType::Private),
            WakeReason::Command(AgentCommand::Digest(3))
        );
    }
}
