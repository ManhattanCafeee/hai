use super::{
    account::account_element, chat::render_chat_info, memory::memory_element,
    message::conversation_element, perception::perception_item, topic::topic_element,
};
use crate::{
    agent::{context::RenderContext, event::EventGroup},
    agentcore::render::{Node, render_xml},
    domain::model::Memory,
};

/// Renders one `<context>` root, shared by the first and later turns of a chapter.
pub fn render_main_context(ctx: &RenderContext, instruction: Node) -> String {
    render_xml(
        &Node::tag("context").children(
            ContextBuilder::new(ctx, instruction)
                .env()
                .chat()
                .accounts()
                .current()
                .recall()
                .perceptions()
                .conversation()
                .build(),
        ),
    )
}

pub(crate) fn render_situation_only(groups: &[EventGroup]) -> String {
    render_xml(&Node::tag("context").child(build_situation_section(groups)))
}

pub fn build_situation_section(groups: &[EventGroup]) -> Node {
    if groups.is_empty() {
        return Node::tag("situation");
    }

    let children: Vec<Node> = groups
        .iter()
        .map(|g| {
            let mut n = Node::tag("trigger")
                .attr("reason", g.label)
                .child(Node::text(&g.describe));
            if g.count > 1 {
                n = n.attr("count", g.count.to_string());
            }
            n
        })
        .collect();

    Node::tag("situation").children(children)
}

struct ContextBuilder<'a> {
    ctx: &'a RenderContext,
    children: Vec<Node>,
}

impl<'a> ContextBuilder<'a> {
    fn new(ctx: &'a RenderContext, instruction: Node) -> Self {
        ContextBuilder {
            ctx,
            children: vec![instruction],
        }
    }

    fn add(mut self, node: Node) -> Self {
        if !node.is_empty() {
            self.children.push(node);
        }
        self
    }

    fn env(self) -> Self {
        let mut env = Node::tag("environment");

        if self.ctx.first_turn {
            env = env.child(
                Node::tag("you_are")
                    .attr("id", self.ctx.channel.account_id.0)
                    .attr("username", &self.ctx.channel.username)
                    .attr("name", &self.ctx.channel.name),
            );
        }
        env = env.child(Node::tag("current_time").child(Node::text(&self.ctx.current_time)));

        self.add(env)
    }

    fn chat(self) -> Self {
        if !self.ctx.first_turn {
            return self;
        }
        let chat = self
            .ctx
            .chat
            .as_ref()
            .expect("a chapter-opening turn carries the chat row");
        let node = render_chat_info(chat);
        self.add(node)
    }

    fn accounts(self) -> Self {
        if !self.ctx.first_turn {
            return self;
        }
        let accounts: Vec<Node> = self
            .ctx
            .accounts
            .iter()
            .filter(|a| a.id != self.ctx.channel.account_id)
            .map(account_element)
            .collect();
        self.add(Node::tag("accounts").children(accounts))
    }

    fn current(self) -> Self {
        let ctx = self.ctx;
        let cutoff =
            jiff::Timestamp::now() - jiff::SignedDuration::from_hours(ctx.topic_idle_hours);
        let (active, stale): (Vec<_>, Vec<_>) = ctx
            .current_topics
            .iter()
            .partition(|topic| topic.last_active_at >= cutoff);

        let topics: Vec<Node> = active
            .iter()
            .map(|topic| topic_element(topic, false))
            .chain(stale.iter().map(|topic| topic_element(topic, true)))
            .collect();
        self.topics_and_memories("current", topics, &ctx.pinned_memories)
    }

    fn recall(self) -> Self {
        let ctx = self.ctx;
        let topics: Vec<Node> = ctx
            .recalled_topics
            .iter()
            .map(|topic| topic_element(topic, false))
            .collect();
        self.topics_and_memories("recall", topics, &ctx.recalled_memories)
    }

    fn topics_and_memories(self, tag: &str, topics: Vec<Node>, memories: &[Memory]) -> Self {
        let memories: Vec<Node> = memories.iter().map(memory_element).collect();
        if topics.is_empty() && memories.is_empty() {
            return self;
        }
        self.add(Node::tag(tag).children(topics).children(memories))
    }

    fn perceptions(self) -> Self {
        let nodes: Vec<Node> = self.ctx.perceptions().iter().map(perception_item).collect();
        self.add(Node::tag("perceptions").children(nodes))
    }

    fn conversation(self) -> Self {
        let msg_refs: Vec<&_> = self.ctx.messages.window().iter().collect();
        let node = conversation_element(&msg_refs, self.ctx);
        self.add(node)
    }

    fn build(self) -> Vec<Node> {
        self.children
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::{
            context::render_context::fixtures::render_context_data,
            event::{WakeEvent, WakeEvents, WakeReason},
        },
        agentcore::{Language, TextCatalog, render::elements::Node},
        domain::{
            model::{Account, Memory, Platform, Topic, TopicStatus},
            test_support,
            vo::{AccountId, AgentCommand, ChatId},
        },
    };

    #[test]
    fn situation_only_prompt_carries_the_wake_instruction() {
        let groups = [EventGroup {
            label: "scheduled",
            describe: "remind the user to take their medication".to_owned(),
            count: 1,
        }];
        let rendered = render_situation_only(&groups);

        assert!(rendered.contains("<situation>"), "{rendered}");
        assert!(rendered.contains("reason=\"scheduled\""), "{rendered}");
        assert!(
            rendered.contains("remind the user to take their medication"),
            "{rendered}"
        );
    }

    /// A command wake stores no message, so the instruction it carries has to be the whole turn.
    #[test]
    fn a_command_wake_renders_its_instruction_without_messages() {
        let reason = WakeReason::Command(AgentCommand::Digest(3));
        let instruction = AgentCommand::Digest(3).instruction(TextCatalog::Zh);
        let events = WakeEvents::new(vec![WakeEvent::new(reason.clone())]);

        let rendered = render_situation_only(&events.coalesce(Language::Zh));

        assert!(
            rendered.contains(&format!("reason=\"{}\"", reason.label())),
            "{rendered}"
        );
        assert!(rendered.contains(&instruction), "{rendered}");
    }

    fn memory(id: u128, content: &str) -> Memory {
        Memory {
            chat_id: ChatId(2),
            created_at: jiff::Timestamp::now(),
            updated_at: jiff::Timestamp::now(),
            ..test_support::memory(id, content)
        }
    }

    fn topic(id: u128, title: &str, idle_hours: i64) -> Topic {
        let at = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(idle_hours);
        Topic {
            chat_id: ChatId(2),
            status: TopicStatus::Active,
            started_at: at,
            last_active_at: at,
            created_at: at,
            updated_at: at,
            ..test_support::topic(id, title)
        }
    }

    /// `<current>` holds the agent's own marks (open topics plus pinned memories) and `<recall>` holds
    /// what the system dredged; an idle-past-timeout topic inside `<current>` carries need-close.
    #[test]
    fn injection_block_splits_current_and_recall() {
        let mut data = render_context_data();
        data.current_topics = vec![
            topic(1, "what we're discussing", 0),
            topic(2, "something left hanging", 5),
        ];
        data.pinned_memories = vec![memory(10, "standing fact")];
        data.recalled_memories = vec![memory(11, "recalled memory")];
        data.recalled_topics = vec![topic(3, "something from before", 48)];

        let ctx = RenderContext::new(data);

        let rendered = render_main_context(&ctx, Node::tag("situation"));

        assert!(rendered.contains("<current>"), "{rendered}");
        assert!(rendered.contains("<recall>"), "{rendered}");
        assert!(rendered.contains("standing fact"), "{rendered}");
        assert!(rendered.contains("what we're discussing"), "{rendered}");
        assert!(rendered.contains("recalled memory"), "{rendered}");
        assert!(rendered.contains("something from before"), "{rendered}");
        // an idle-past-timeout topic carries need-close; an active one does not
        assert_eq!(rendered.matches("need-close").count(), 1, "{rendered}");
        // system-dredged entries stay in the recall section, not in current
        let current = rendered
            .split("<recall>")
            .next()
            .expect("current section before recall");
        assert!(!current.contains("recalled memory"), "{rendered}");
        assert!(!current.contains("something from before"), "{rendered}");
    }

    /// A later turn of a chapter renders no identity, profile or directory (every item appears once in the
    /// accumulated context) and refreshes current_time only.
    #[test]
    fn later_turn_renders_only_the_volatile_parts() {
        let data = |first_turn: bool| {
            let mut data = render_context_data();
            data.chat.as_mut().expect("fixture chat").name = Some("Test Group".into());
            data.current_time = "2026-09-23T20:21:30+08:00".to_owned();
            data.first_turn = first_turn;
            data.accounts = vec![Account {
                id: AccountId(6),
                identity_id: None,
                platform: Platform::Telegram,
                external_id: "6".into(),
                meta: None,
                last_active_at: jiff::Timestamp::now(),
                created_at: jiff::Timestamp::now(),
                updated_at: jiff::Timestamp::now(),
            }];
            data
        };

        let first = render_main_context(&RenderContext::new(data(true)), Node::tag("situation"));
        assert!(first.contains("<you_are"), "{first}");
        assert!(first.contains("<accounts>"), "{first}");
        // an attribute-only node (chat) must not be dropped as empty
        assert!(first.contains("<chat id=\"2\""), "{first}");
        assert!(first.contains("name=\"Test Group\""), "{first}");

        let later = render_main_context(&RenderContext::new(data(false)), Node::tag("situation"));
        assert!(!later.contains("<you_are"), "{later}");
        assert!(!later.contains("<accounts>"), "{later}");
        assert!(!later.contains("<chat "), "{later}");
        assert!(later.contains("<current_time>"), "{later}");
    }
}
