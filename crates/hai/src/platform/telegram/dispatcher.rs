//! Routing one Telegram update: the filters say what arrived, the arrival builder says what it
//! means, and everything after that is the inbound tail's.

use std::sync::Arc;

use teloxide::{
    Bot,
    dispatching::{Dispatcher, ShutdownToken, UpdateFilterExt},
    dptree,
    types::{Me, Message, Update},
};
use tokio::sync::oneshot;

use super::arrival::{self, BotIdentity};
use crate::{agent::intake::Intake, domain::repo::ChangeKind, error::Result};

pub(super) struct TelegramDispatcher {
    bot: Bot,
    intake: Arc<Intake>,
}

impl TelegramDispatcher {
    pub(super) async fn new(bot: Bot, intake: Arc<Intake>) -> Self {
        Self { bot, intake }
    }

    pub(super) async fn run(self, shutdown_tx: oneshot::Sender<ShutdownToken>) {
        let this = Arc::new(self);

        let dispatcher_handler = dptree::entry()
            .branch(Update::filter_message().endpoint(
                |msg: Message, me: Me, dp: Arc<TelegramDispatcher>| async move {
                    dp.handle_message(msg, me).await
                },
            ))
            .branch(Update::filter_edited_message().endpoint(
                |msg: Message, me: Me, dp: Arc<TelegramDispatcher>| async move {
                    dp.handle_edited(msg, me).await
                },
            ));

        // Do not enable teloxide's `enable_ctrlc_handler`: it registers a second
        // SIGINT listener that shuts the dispatcher down on its own, cancelling the
        // in-flight GetUpdates long poll — which teloxide's default error handler
        // then reports as a spurious ERROR instead of a clean stop.
        let mut dispatcher = Dispatcher::builder(this.bot.clone(), dispatcher_handler)
            .dependencies(dptree::deps![Arc::clone(&this)])
            .build();
        let _ = shutdown_tx.send(dispatcher.shutdown_token());
        dispatcher.dispatch().await;
    }

    async fn handle_message(&self, msg: Message, me: Me) -> Result<()> {
        let Some(from) = msg.from.as_ref() else {
            return Ok(());
        };
        tracing::debug!(
            chat_id = %msg.chat.id,
            from = %from.full_name(),
            "Message received",
        );
        // A message the bot first sees after it was edited still carries the platform's edit time.
        let change = ChangeKind::Created {
            edited_at: edited_at(&msg)?,
        };
        self.deliver(&msg, &me, change).await
    }

    /// An edit is a change, not a command, and only its sender can edit a message — so a message the
    /// bot sent can only have been edited by the bot.
    async fn handle_edited(&self, msg: Message, me: Me) -> Result<()> {
        let Some(from) = msg.from.as_ref() else {
            return Ok(());
        };
        if from.id == me.user.id {
            return Ok(());
        }
        let change = arrival::change_kind(true, edited_at(&msg)?)?;
        self.deliver(&msg, &me, change).await
    }

    /// One update read into an arrival and handed to the inbound tail; what the platform made of it
    /// is dropped when it belongs to another bot.
    async fn deliver(&self, msg: &Message, me: &Me, change: ChangeKind) -> Result<()> {
        let bot = BotIdentity {
            username: me.user.username.as_deref().unwrap_or_default(),
            user_id: me.user.id,
        };
        match arrival::arrival(msg, &bot, change)? {
            Some(arrival) => self.intake.accept(arrival).await,
            None => {
                tracing::debug!(
                    chat_id = %msg.chat.id,
                    "Slash command addressed to another bot, ignored",
                );
                Ok(())
            }
        }
    }
}

/// The platform's own edit time, when it reports one.
fn edited_at(msg: &Message) -> Result<Option<jiff::Timestamp>> {
    msg.edit_date()
        .map(|at| jiff::Timestamp::from_second(at.timestamp()))
        .transpose()
        .map_err(Into::into)
}
