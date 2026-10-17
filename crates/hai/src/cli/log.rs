use strum::IntoEnumIterator;

use crate::{
    config::AppConfig,
    domain::{
        db,
        repo::Repos,
        vo::{ChatId, EventKind},
    },
    error::AppError,
};

pub async fn execute(args: super::LogArgs, cfg: &AppConfig) -> crate::error::Result<()> {
    let kind = parse_kind(args.event.as_deref())?;
    if args.chat == Some(0) {
        return Err(AppError::message("chat 0 is not a chat id".to_owned()));
    }
    let pool = db::init_db(&cfg.database).await?;
    let repos = Repos::new(pool);

    if let Some(seq) = args.id {
        show_detail(seq, &repos).await?;
    } else {
        super::tui::run_tui(repos, args.chat.map(ChatId), kind).await?;
    }

    Ok(())
}

/// The `--event` filter as a typed kind; an unknown name fails here rather than quietly matching
/// nothing.
fn parse_kind(raw: Option<&str>) -> crate::error::Result<Option<EventKind>> {
    raw.map(|raw| {
        raw.parse::<EventKind>().map_err(|_| {
            let kinds: Vec<&str> = EventKind::iter().map(EventKind::as_str).collect();
            AppError::message(format!(
                "unknown event kind {raw:?}, expected one of {}",
                kinds.join(", ")
            ))
        })
    })
    .transpose()
}

async fn show_detail(seq: i64, repos: &Repos) -> crate::error::Result<()> {
    use super::display::{self, EventDisplay};

    let Some(event) = repos.event.by_seq(seq).await? else {
        eprintln!("Event #{seq} not found");
        return Ok(());
    };

    let Some(d) = EventDisplay::from_event(&event) else {
        eprintln!("Event #{seq}: unable to deserialize");
        return Ok(());
    };
    println!(
        "{}  {:+}  {}",
        display::fmt_time(event.created_at),
        d.chat_id,
        d.tag
    );
    println!("────────────────────────────────────────────────────────────────────────");
    println!("{}", display::render_sections(&d.sections, false));

    Ok(())
}
