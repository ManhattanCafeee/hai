use std::collections::{HashMap, HashSet};

use genai::chat::ChatMessage;

use crate::{
    agent::{
        context::{
            ContextMessages, RenderContext, build_situation_section,
            load::{
                ChapterRecall, LoadedPerceptions, chapter_recall, collect_accounts, load_chat,
                load_perceptions, load_reply_map,
            },
            render_context::RenderContextData,
            render_main_context, render_situation_only,
            types::BuiltContext,
        },
        runtime::turn::TurnContext,
    },
    domain::{
        model::{Account, Attachment, Chat, Memory, Message, Topic},
        vo::MessageId,
    },
    error::Result,
};

struct PromptInput {
    messages: ContextMessages,
    attachments: HashMap<MessageId, Vec<Attachment>>,
    perception: LoadedPerceptions,
    topics: Vec<Topic>,
    pinned_memories: Vec<Memory>,
    window_text: String,
    accounts: Vec<Account>,
    chat: Option<Chat>,
}

/// Loads every input `build_prompt` needs.
async fn prepare_prompt_input(
    ctx: &TurnContext,
    messages: &[Message],
    is_first: bool,
) -> Result<PromptInput> {
    let services = &ctx.app.services;
    let chat_id = ctx.tool.chat.chat_id();

    let reply_map = load_reply_map(services, messages).await?;

    // Out-of-window reply targets come after the window, in id order: perception loading ties
    // each subject to the first occurrence it sees, so that order has to be stable.
    let mut ordered_ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
    let mut reply_ids: Vec<MessageId> = reply_map.keys().copied().collect();
    reply_ids.sort_unstable();
    ordered_ids.extend(reply_ids);

    let attachments = services
        .message
        .attachments_by_messages(&ordered_ids)
        .await?;
    let window_attachments: Vec<Attachment> = ordered_ids
        .iter()
        .filter_map(|id| attachments.get(id))
        .flatten()
        .cloned()
        .collect();
    let texts: Vec<String> = messages
        .iter()
        .map(|message| {
            let own = attachments.get(&message.id).map_or(&[][..], Vec::as_slice);
            message.content(own)
        })
        .collect();
    let perception = load_perceptions(services, &window_attachments, &texts, is_first).await?;

    let (topics, pinned_memories, chat) = if is_first {
        (
            services.topic.active_topics(chat_id).await?,
            services
                .memory
                .pinned(chat_id, ctx.app.cfg.agent.context.pinned_memory_limit)
                .await?,
            Some(load_chat(services, chat_id).await?),
        )
    } else {
        (Vec::new(), Vec::new(), None)
    };
    let accounts = collect_accounts(services, messages, &reply_map).await?;

    Ok(PromptInput {
        messages: ContextMessages::new(messages.to_vec(), reply_map),
        attachments,
        perception,
        topics,
        pinned_memories,
        window_text: texts.join("\n"),
        accounts,
        chat,
    })
}

/// Builds the turn's context prompt: the first turn of a chapter renders the opening blocks, later turns only the volatile sections.
pub async fn build_prompt(
    ctx: &TurnContext,
    messages: &[Message],
    is_first: bool,
    new_message_ids: &HashSet<MessageId>,
) -> Result<BuiltContext> {
    let cfg = &ctx.app.cfg;

    if !is_first && messages.is_empty() {
        let rendered = render_situation_only(&ctx.events.coalesce(ctx.language));
        return Ok(BuiltContext {
            messages: vec![ChatMessage::user(&rendered)],
            rendered_prompt: rendered,
        });
    }

    let data = prepare_prompt_input(ctx, messages, is_first).await?;

    let recalled = if is_first {
        chapter_recall(
            &ctx.app.services,
            ctx.tool.chat.chat_id(),
            &data.window_text,
            &data.pinned_memories,
            &data.topics,
        )
        .await?
    } else {
        ChapterRecall::default()
    };

    let context_data = RenderContextData {
        channel: ctx.tool.chat.profile(),
        chat: data.chat,
        current_time: jiff::Zoned::now().to_string(),
        first_turn: is_first,
        messages: data.messages,
        new_message_ids: new_message_ids.clone(),
        attachments_by_message: data.attachments,
        perception_by_attachment: data.perception.by_attachment,
        same_resource_as: data.perception.same_resource_as,
        accounts: data.accounts,
        current_topics: data.topics,
        pinned_memories: data.pinned_memories,
        recalled_memories: recalled.memories,
        recalled_topics: recalled.topics,
        perceptions: data.perception.url_items,
        topic_idle_hours: cfg.agent.context.topic_idle_hours,
    };
    let render_ctx = RenderContext::new(context_data);
    let instruction = build_situation_section(&ctx.events.coalesce(ctx.language));

    let rendered = render_main_context(&render_ctx, instruction);

    Ok(BuiltContext {
        messages: vec![ChatMessage::user(&rendered)],
        rendered_prompt: rendered,
    })
}
