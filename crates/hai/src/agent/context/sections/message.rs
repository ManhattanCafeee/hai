use crate::{
    agent::context::{
        RenderContext,
        fmt::{format_date_label, format_time_only},
        sections::attachment::attachment_element,
    },
    agentcore::render::elements::Node,
    domain::model::Message,
};

pub(crate) fn message_element(msg: &Message, ctx: &RenderContext) -> Node {
    let own = msg.account_id == Some(ctx.channel.account_id);

    let mut b = Node::tag("msg")
        .attr("id", msg.id.0)
        .attr("from", ctx.sender_name(msg))
        .attr("at", format_time_only(msg.sent_at));

    if let Some(account_id) = msg.account_id {
        b = b.attr("account", account_id.0);
    }

    if own {
        b = b.attr("own", true);
    }

    if let Some(edited_at) = msg.edited_at {
        b = b.attr("edited_at", format_time_only(edited_at));
    }

    let quote = quoted_part(msg);

    if let Some(reply_id) = msg.reply_to_id
        && let Some(replied) = ctx.messages.get(reply_id)
    {
        let replied_msg = replied.message();
        let replied_sender = ctx.sender_name(replied_msg);

        let mut elements = message_content(replied_msg, ctx);
        // An in-window reference is already visible as its own message, so its preview is truncated;
        // an out-of-window one is the only presentation of that message and keeps the full text.
        if replied.is_in_window() {
            truncate_text_nodes(&mut elements, 50);
        }

        let mut reference = Node::tag("reference")
            .attr("id", reply_id.0)
            .attr("from", replied_sender.as_str())
            .attr("at", format_time_only(replied_msg.sent_at));

        if let Some(quote) = &quote {
            reference = reference.child(quote_node(quote));
        }

        reference = reference.children(elements);
        b = b.child(reference);
    } else if let Some(quote) = &quote {
        // A reference outside this chat resolves to no row; the excerpt is still what is being
        // answered.
        b = b.child(quote_node(quote));
    }

    let content_elements = message_content(msg, ctx);
    b.children(content_elements)
}

/// The excerpt this message points at, when its sender chose one: a quote the platform added by
/// itself says nothing the reference preview does not already show.
fn quoted_part(msg: &Message) -> Option<String> {
    let meta = match msg.platform_meta() {
        Ok(meta) => meta,
        Err(e) => {
            tracing::warn!(error = %e, "unreadable message meta; excerpt dropped");
            return None;
        }
    };
    meta?.excerpt().map(str::to_owned)
}

fn quote_node(quote: &str) -> Node {
    Node::tag("quote").child(Node::text(quote))
}

/// One message's rendered content: its attachments in ordinal order, then its own text.
fn message_content(msg: &Message, ctx: &RenderContext) -> Vec<Node> {
    let mut nodes: Vec<Node> = ctx
        .attachments_by_message
        .get(&msg.id)
        .into_iter()
        .flatten()
        .map(|att| attachment_element(att, ctx))
        .collect();
    if !msg.text.is_empty() {
        nodes.push(Node::text(&msg.text));
    }
    nodes
}

pub fn conversation_element(messages: &[&Message], ctx: &RenderContext) -> Node {
    if messages.is_empty() {
        return Node::tag("conversation").attr("total", 0);
    }

    let first_new = messages
        .iter()
        .position(|m| ctx.new_message_ids.contains(&m.id));

    let mut root = Node::tag("conversation").attr("total", messages.len() as i64);
    let mut prev_date: Option<String> = None;

    for (i, msg) in messages.iter().enumerate() {
        let label = format_date_label(msg.sent_at);
        if prev_date.as_ref() != Some(&label) {
            root = root.child(Node::tag("date").attr("value", &label));
            prev_date = Some(label);
        }

        if Some(i) == first_new {
            root = root.child(Node::tag("separator").child(Node::text("new messages")));
        }

        root = root.child(message_element(msg, ctx));
    }

    root
}

fn truncate_text_nodes(elements: &mut [Node], max_chars: usize) {
    for el in elements.iter_mut() {
        if let Node::Text(t) = el
            && t.chars().count() > max_chars
        {
            *t = crate::util::text::truncate_chars(t, max_chars);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{
        agent::context::{ContextMessages, render_context::fixtures::render_context_data},
        agentcore::render::render_xml,
        domain::{
            model::{Attachment, Perception},
            test_support,
            vo::{
                AccountId, AttachmentId, AttachmentKind, AttachmentParser, ChatId, MessageId,
                PerceptionId,
            },
        },
    };

    fn msg(
        id: MessageId,
        account_id: Option<AccountId>,
        reply_to_id: Option<MessageId>,
        text: &str,
    ) -> Message {
        Message {
            id,
            chat_id: ChatId(2),
            account_id,
            reply_to_id,
            sent_at: Some(jiff::Timestamp::now()),
            created_at: jiff::Timestamp::now(),
            updated_at: jiff::Timestamp::now(),
            ..test_support::message(text)
        }
    }

    fn render_ctx(
        messages: Vec<Message>,
        reply_context: HashMap<MessageId, Message>,
    ) -> RenderContext {
        let mut data = render_context_data();
        data.messages = ContextMessages::new(messages, reply_context);
        data.topic_idle_hours = 0;
        RenderContext::new(data)
    }

    fn attachment(id: u128, ordinal: i32, file_unique_id: &str) -> Attachment {
        Attachment {
            id: AttachmentId(uuid::Uuid::from_u128(id)),
            message_id: MessageId(37575),
            ordinal,
            file_unique_id: Some(file_unique_id.to_owned()),
            file_id: format!("handle-{file_unique_id}"),
            caption: Some("cap".into()),
            created_at: jiff::Timestamp::now(),
            ..test_support::attachment(AttachmentKind::Photo)
        }
    }

    fn perception(
        id: u128,
        subject: &str,
        parser: AttachmentParser,
        focus: Option<&str>,
    ) -> Perception {
        Perception {
            id: PerceptionId(uuid::Uuid::from_u128(id)),
            focus: focus.map(str::to_owned),
            created_at: jiff::Timestamp::now(),
            ..test_support::perception(subject, parser, "a cat")
        }
    }

    #[test]
    fn out_of_window_reference_not_rendered_as_message() {
        let long_text = "This quoted content runs well past fifty characters so the out-of-window reference keeps its full text visible in the context.";
        let replied = msg(MessageId(37575), Some(AccountId(6)), None, long_text);
        let reply = msg(
            MessageId(37576),
            Some(AccountId(1)),
            Some(MessageId(37575)),
            "mostly talking about domain renewals lately",
        );
        let sticker = msg(MessageId(37577), Some(AccountId(4)), None, "");

        let ctx = render_ctx(
            vec![reply.clone(), sticker.clone()],
            HashMap::from([(MessageId(37575), replied)]),
        );
        let node = conversation_element(&[&reply, &sticker], &ctx);
        let out = render_xml(&node);

        assert!(out.contains("<reference id=\"37575\""));
        assert!(out.contains(long_text));
        assert!(!out.contains("…"));
        assert!(!out.contains("<msg id=\"37575\""));
        assert!(out.contains("<msg id=\"37576\""));
        assert!(out.contains("<msg id=\"37577\""));
    }

    /// Every message carries its sender's account id, so a person appearing mid-chapter can be referenced
    /// without relying on the first-turn directory.
    #[test]
    fn message_carries_sender_account_id() {
        let message = msg(MessageId(37575), Some(AccountId(6)), None, "hello");
        let ctx = render_ctx(vec![message.clone()], HashMap::new());
        let out = render_xml(&conversation_element(&[&message], &ctx));

        assert!(out.contains("<msg id=\"37575\" from="), "{out}");
        assert!(out.contains("account=\"6\""), "{out}");
    }

    #[test]
    fn in_window_reference_renders_message_and_reference() {
        let long_text = "This quoted content runs well past fifty characters so the in-window reference gets truncated next to its own visible message.";
        let replied = msg(MessageId(37575), Some(AccountId(6)), None, long_text);
        let reply = msg(
            MessageId(37576),
            Some(AccountId(1)),
            Some(MessageId(37575)),
            "replying to that",
        );

        let ctx = render_ctx(vec![replied.clone(), reply.clone()], HashMap::new());
        let node = conversation_element(&[&replied, &reply], &ctx);
        let out = render_xml(&node);

        assert!(out.contains("<msg id=\"37575\""));
        assert!(out.contains("<reference id=\"37575\""));
        assert!(out.contains(long_text));
        assert!(out.contains("…"));
    }

    /// Attachments render in ordinal order ahead of the message text, with the cached analyses inlined;
    /// a repeated file points at the attachment that carried the analysis first.
    #[test]
    fn attachments_render_before_the_text_with_their_analyses() {
        let message = msg(MessageId(37575), Some(AccountId(6)), None, "look at these");
        let first = attachment(1, 0, "uniq");
        let repeat = attachment(2, 1, "uniq");

        let mut ctx = render_ctx(vec![message.clone()], HashMap::new());
        ctx.data
            .attachments_by_message
            .insert(message.id, vec![first.clone(), repeat.clone()]);
        ctx.data.perception_by_attachment.insert(
            first.id,
            vec![
                perception(9, "telegram:uniq", AttachmentParser::Image, None),
                perception(
                    10,
                    "telegram:uniq",
                    AttachmentParser::Image,
                    Some("what breed?"),
                ),
            ],
        );
        ctx.data.same_resource_as.insert(repeat.id, first.id);

        let out = render_xml(&message_element(&message, &ctx));

        let attachments_at = out
            .find(&format!("<attachment id=\"{}\"", first.id))
            .expect("first attachment");
        let text_at = out.find("look at these").expect("message text");
        assert!(attachments_at < text_at, "{out}");
        assert!(out.contains("type=\"photo\""), "{out}");
        assert!(out.contains("caption=\"cap\""), "{out}");
        assert!(out.contains("<analysis parser=\"image\">"), "{out}");
        assert!(out.contains("focus=\"what breed?\""), "{out}");
        assert!(out.contains("a cat"), "{out}");
        // the repeat carries no analysis of its own, only the pointer at the first occurrence
        assert_eq!(out.matches("<analysis").count(), 2, "{out}");
        assert!(
            out.contains("same_resource_as=\"00000000-0000-0000-0000-000000000001\""),
            "{out}"
        );
    }

    /// An edited message says so: a correction reads differently from the original wording.
    #[test]
    fn an_edited_message_carries_its_edit_time() {
        let mut message = msg(MessageId(37575), Some(AccountId(6)), None, "hello");

        let plain = render_xml(&conversation_element(
            &[&message],
            &render_ctx(vec![message.clone()], HashMap::new()),
        ));
        assert!(!plain.contains("edited_at"), "{plain}");

        message.edited_at = Some(jiff::Timestamp::now());
        let edited = render_xml(&conversation_element(
            &[&message],
            &render_ctx(vec![message.clone()], HashMap::new()),
        ));
        assert!(edited.contains("edited_at=\""), "{edited}");
    }

    /// The divider marks where this turn's batch starts: it is placed from the batch's message ids,
    /// so what sits above it is history the chapter has already read.
    #[test]
    fn the_divider_sits_at_the_first_message_of_the_batch() {
        let history = msg(MessageId(1), Some(AccountId(6)), None, "earlier");
        let fresh = msg(MessageId(2), Some(AccountId(6)), None, "just now");
        let mut ctx = render_ctx(vec![history.clone(), fresh.clone()], HashMap::new());
        ctx.data.new_message_ids.insert(fresh.id);

        let out = render_xml(&conversation_element(&[&history, &fresh], &ctx));
        let divider = out.find("new messages").expect("a divider");
        let first = out.find("<msg id=\"1\"").expect("the history message");
        let second = out.find("<msg id=\"2\"").expect("the batch message");
        assert!(first < divider && divider < second, "{out}");
    }

    /// A reference the person narrowed to an excerpt shows that excerpt; a quote the server added by
    /// itself would only repeat the preview beside it.
    #[test]
    fn a_chosen_excerpt_renders_inside_its_reference() {
        let original = msg(
            MessageId(37575),
            Some(AccountId(6)),
            None,
            "the whole long original",
        );
        let mut reply = msg(
            MessageId(37576),
            Some(AccountId(1)),
            Some(MessageId(37575)),
            "about that",
        );
        let quote = |is_manual: bool| {
            serde_json::json!({
                "platform": "telegram",
                "quote": { "text": "whole long", "position": 4, "is_manual": is_manual },
            })
        };

        reply.meta = Some(quote(true));
        let ctx = render_ctx(vec![original.clone(), reply.clone()], HashMap::new());
        let out = render_xml(&conversation_element(&[&original, &reply], &ctx));
        assert!(out.contains("<quote"), "{out}");
        assert!(out.contains("whole long"), "{out}");

        reply.meta = Some(quote(false));
        let ctx = render_ctx(vec![original.clone(), reply.clone()], HashMap::new());
        let out = render_xml(&conversation_element(&[&original, &reply], &ctx));
        assert!(!out.contains("<quote"), "{out}");
    }

    /// An unreadable meta drops the excerpt instead of failing the render; the message itself still
    /// shows.
    #[test]
    fn unreadable_meta_drops_the_quote_child() {
        let original = msg(
            MessageId(37575),
            Some(AccountId(6)),
            None,
            "the whole long original",
        );
        let mut reply = msg(
            MessageId(37576),
            Some(AccountId(1)),
            Some(MessageId(37575)),
            "about that",
        );
        reply.meta = Some(serde_json::json!("not a message meta"));

        let ctx = render_ctx(vec![original.clone(), reply.clone()], HashMap::new());
        let out = render_xml(&conversation_element(&[&original, &reply], &ctx));
        assert!(!out.contains("<quote"), "{out}");
        assert!(out.contains("about that"), "{out}");
    }
}
