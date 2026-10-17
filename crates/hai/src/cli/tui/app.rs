use std::time::Duration;

use crossterm::event::{Event as TermEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    macros::line,
    prelude::Stylize,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};

use super::{
    action::{self, Action, Effect, Mode, Pane},
    chats::Chats,
    detail::{Detail, DetailLayout},
    event_store::EventStore,
    filter::Query,
    guard::{InputReader, TerminalGuard},
    timeline::Timeline,
};
use crate::{
    domain::{
        repo::Repos,
        vo::{ChatId, EventKind},
    },
    error::Result,
};

const POLL_MS: u64 = 200;

/// The event log as the reader walks it: the window and its cached renderings (`EventStore`), the
/// query line, and the three views over the window.
pub(super) struct App {
    store: EventStore,
    query: Query,
    chats: Chats,
    timeline: Timeline,
    detail: Detail,
    pane: Pane,
    help_open: bool,
    /// What failed in the loop; a query that does not parse carries its own message.
    error: Option<String>,
    quit: bool,
    dirty: bool,
}

impl App {
    fn new(repos: Repos, chat_filter: Option<ChatId>, kind_filter: Option<EventKind>) -> Self {
        Self {
            store: EventStore::new(repos, chat_filter, kind_filter),
            query: Query::new(seed_query(chat_filter, kind_filter)),
            chats: Chats::new(),
            timeline: Timeline::new(),
            detail: Detail::new(),
            pane: Pane::Timeline,
            help_open: false,
            error: None,
            quit: false,
            dirty: false,
        }
    }

    async fn load_initial(&mut self) -> Result<()> {
        self.store.load_end().await?;
        self.refresh();
        self.timeline.follow_end();
        Ok(())
    }

    /// Re-derive every view from the window: the one place that knows how the three depend on each
    /// other.
    fn refresh(&mut self) {
        let displays = self.store.displays();
        self.timeline.rebuild(&displays, &self.query.parsed);
        self.chats
            .rebuild(self.timeline.groups(), self.query.parsed.chat);
        self.detail.keep_in_window(&self.store);
    }

    /// Everything an action does to the state, and the work it asks of the store. The effects come
    /// back for `apply` to run, which is what keeps the database off the key path.
    fn update(&mut self, action: Action) -> Vec<Effect> {
        let mut effects = Vec::new();
        match action {
            Action::Quit => self.quit = true,
            Action::ToggleHelp => self.help_open = !self.help_open,
            Action::CyclePane(delta) => self.pane = self.pane.cycle(delta, self.detail.is_open()),
            Action::OpenQuery => self.query.begin(),
            Action::CloseQuery => self.query.commit(),
            Action::CancelQuery => {
                self.query.cancel();
                effects.extend(self.query_reload());
            }
            Action::Move(delta) => self.move_in_pane(delta),
            Action::Page(dir) => self.page_in_pane(dir, false),
            Action::HalfPage(dir) => self.page_in_pane(dir, true),
            Action::Act => {
                self.act();
                effects.extend(self.query_reload());
            }
            Action::Cancel => self.cancel(),
            Action::JumpHead => {
                let at_head =
                    self.timeline.selected_index() == 0 && self.timeline.selected.is_some();
                if at_head && !self.store.at_start {
                    effects.push(Effect::LoadOlder);
                } else {
                    self.timeline.select_at(0);
                }
            }
            Action::JumpTail => effects.push(Effect::LoadEnd),
            Action::ToggleFollow => self.timeline.follow = !self.timeline.follow,
            Action::ToggleFullscreen => self.detail.toggle_layout(),
            Action::QueryKey(key) => {
                self.query.handle_key(key);
                effects.extend(self.query_reload());
            }
        }
        self.refresh();
        effects
    }

    /// What owns the keyboard right now.
    fn mode(&self) -> Mode {
        if self.help_open {
            Mode::Help
        } else if self.query.is_editing() {
            Mode::Query
        } else {
            Mode::Panes(self.pane)
        }
    }

    /// A term the database narrows by has to be re-read the moment it changes; the rest filter what is
    /// already in memory.
    fn query_reload(&self) -> Option<Effect> {
        self.query
            .stale_sql(
                self.store.chat_filter.map(|chat| chat.0),
                self.store.kind_filter,
            )
            .then_some(Effect::Reload)
    }

    fn move_in_pane(&mut self, delta: isize) {
        match self.pane {
            Pane::Chats => self.chats.select_rel(delta),
            Pane::Timeline => {
                self.timeline.select_rel(delta);
                self.follow_detail();
            }
            Pane::Detail => self.detail.scroll(delta, &mut self.store),
        }
    }

    /// Move the focused pane's cursor by a page, or by half of one where it counts them smaller.
    fn page_in_pane(&mut self, dir: isize, half: bool) {
        match self.pane {
            Pane::Chats => self.chats.page(dir, half),
            Pane::Timeline => {
                self.timeline.page(dir, half);
                self.follow_detail();
            }
            Pane::Detail => self.detail.page(dir, half, &mut self.store),
        }
    }

    /// The detail, while it is open, shows whatever the timeline has selected.
    fn follow_detail(&mut self) {
        if self.detail.is_open() {
            self.detail.show(self.timeline.selected_seq());
        }
    }

    /// The focused pane's primary action.
    fn act(&mut self) {
        match self.pane {
            Pane::Chats => {
                if let Some(chat) = self.chats.selected_chat() {
                    self.query.toggle_chat(chat);
                }
            }
            Pane::Timeline => {
                let Some(row) = self.timeline.selected_row() else {
                    return;
                };
                let opens = self
                    .timeline
                    .group_of(row)
                    .is_some_and(|group| group.turn.is_some());
                if !row.child && opens {
                    self.timeline.toggle(row);
                } else {
                    self.detail.show(Some(row.seq));
                    self.pane = Pane::Detail;
                }
            }
            Pane::Detail => self.detail.toggle_unfold(),
        }
    }

    /// Back out of the closest thing that is open: the detail, or the message on the error line. The
    /// help and the query line are modes of their own and handle their own `Esc`.
    fn cancel(&mut self) {
        if self.detail.is_open() {
            self.detail.close();
            if self.pane == Pane::Detail {
                self.pane = Pane::Timeline;
            }
        } else {
            self.error = None;
        }
    }

    fn set_error(&mut self, message: String) {
        if self.error.is_none() {
            self.error = Some(message);
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        use ratatui::{Terminal, backend::CrosstermBackend};

        let _guard = TerminalGuard::enter()?;
        let mut input = InputReader::spawn()?;
        let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;

        if let Err(e) = self.load_initial().await {
            self.error = Some(format!("load: {e}"));
        }

        loop {
            // Input first: a key is handled the moment it lands, and the log's tail is looked at only
            // once the reader has been quiet for a poll interval. A competing timer branch starves
            // input instead — a burst of keys then drains at the tick's pace rather than as it lands.
            match tokio::time::timeout(Duration::from_millis(POLL_MS), input.next()).await {
                Ok(Some(event)) => {
                    self.handle_terminal(event).await;
                    self.dirty = true;
                }
                // The reader stopped, so nothing can wake this loop again.
                Ok(None) => break,
                Err(_) => {
                    self.poll().await;
                    self.dirty = true;
                }
            }
            if self.quit {
                break;
            }
            if self.dirty {
                terminal.draw(|f| self.render(f))?;
                self.dirty = false;
            }
        }
        Ok(())
    }

    /// One terminal event: a key through the keymap, the wheel as a move in the focused pane. Anything
    /// else — a resize, a paste — is left to the next frame.
    async fn handle_terminal(&mut self, event: TermEvent) {
        let action = match event {
            TermEvent::Key(key) => action::key_event(key, self.mode()),
            TermEvent::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => Some(Action::Move(1)),
                MouseEventKind::ScrollUp => Some(Action::Move(-1)),
                _ => None,
            },
            _ => None,
        };
        if let Some(action) = action {
            let effects = self.update(action);
            self.apply(effects).await;
        }
    }

    /// Run the effects an action asked for. A prepend moves every row, so the viewport re-anchors on
    /// the row the reader was looking at once the new window is in place.
    async fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            let anchor = (effect == Effect::LoadOlder).then(|| self.timeline.visual_pos());
            self.load(effect).await;
            self.refresh();
            if let Some(visual) = anchor {
                self.timeline.anchor_after_prepend(visual);
            }
        }
    }

    async fn load(&mut self, effect: Effect) {
        match effect {
            Effect::Reload => {
                self.store.chat_filter = self.query.parsed.chat.map(ChatId);
                self.store.kind_filter = self.query.parsed.kind;
                match self.store.load_end().await {
                    Ok(()) => self.timeline.follow_end(),
                    Err(e) => self.set_error(format!("reload: {e}")),
                }
            }
            Effect::LoadEnd => match self.store.load_end().await {
                Ok(()) => self.timeline.follow_end(),
                Err(e) => self.set_error(format!("load end: {e}")),
            },
            Effect::LoadOlder => {
                self.timeline.back_loading = true;
                match self.store.extend_back().await {
                    Ok(_) => {}
                    Err(e) => self.set_error(format!("load older: {e}")),
                }
                self.timeline.back_loading = false;
            }
        }
    }

    /// One tick: follow the tail, or count what is waiting beyond the window — and, with the reader
    /// parked at the oldest row, pull in the page before it.
    async fn poll(&mut self) {
        if self.timeline.follow {
            match self.store.append_new().await {
                Ok(0) => {}
                Ok(_) => {
                    self.refresh();
                    self.timeline.follow_end();
                }
                Err(e) => self.set_error(format!("append: {e}")),
            }
        }
        match self.store.count_new().await {
            Ok(count) => self.timeline.new_count = count,
            Err(e) => self.set_error(format!("poll: {e}")),
        }
        let at_head = self.timeline.selected_index() == 0 && self.timeline.selected.is_some();
        if at_head
            && !self.store.at_start
            && !self.timeline.back_loading
            && !self.timeline.is_empty()
        {
            self.apply(vec![Effect::LoadOlder]).await;
        }
    }

    fn render(&mut self, f: &mut Frame) {
        self.store.set_viewport(self.timeline.vp_h());

        let error = self.query.error.clone().or_else(|| self.error.clone());
        let error_height = u16::from(error.is_some());
        // The query line is not part of the TUI: it is a row that appears while it has the keyboard.
        let query_height = u16::from(self.query.is_editing());
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(error_height),
            Constraint::Length(query_height),
            Constraint::Length(1),
        ])
        .split(f.area());
        let Some([header, body, error_line, query_line, status]) = chunks.first_chunk::<5>() else {
            return;
        };

        self.render_header(f, *header);
        self.render_body(f, *body);
        if let Some(message) = &error {
            self.render_error(f, *error_line, message);
        }
        if query_height > 0 {
            self.query.render(f, *query_line);
        }
        self.render_status(f, *status);

        if self.help_open {
            self.render_help(f, f.area());
        }
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let window = match (self.store.min_seq(), self.store.max_seq()) {
            (Some(min), Some(max)) => format!("{min}–{max}"),
            _ => String::from("empty"),
        };
        let mut spans = vec![
            Span::styled(
                format!(" hai  {window} / {} events", self.store.window().len()),
                Style::new().bold(),
            ),
            Span::raw("  "),
            Span::styled(
                self.query.parsed.spelling(),
                Style::new().fg(Color::DarkGray),
            ),
        ];
        if self.timeline.new_count > 0 {
            spans.push(Span::styled(
                format!("   +{} new (G to load)", self.timeline.new_count),
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled(
            if self.timeline.follow {
                "   ▶"
            } else {
                "   ⏸"
            },
            Style::new().fg(Color::DarkGray),
        ));
        f.render_widget(Line::from(spans), area);
    }

    fn render_body(&mut self, f: &mut Frame, area: Rect) {
        let detail_open = self.detail.is_open();
        if detail_open && self.detail.layout == DetailLayout::Full {
            self.detail
                .render(f, area, &mut self.store, self.pane == Pane::Detail);
            return;
        }

        let chats = Constraint::Percentage(25);
        let chunks = if detail_open {
            Layout::horizontal([
                chats,
                Constraint::Percentage(38),
                Constraint::Percentage(37),
            ])
            .split(area)
        } else {
            Layout::horizontal([chats, Constraint::Percentage(75)]).split(area)
        };

        match chunks.as_ref() {
            [chats_area, timeline_area] => {
                self.chats.render(f, *chats_area, self.pane == Pane::Chats);
                self.timeline.render(
                    f,
                    *timeline_area,
                    &mut self.store,
                    self.pane == Pane::Timeline,
                );
            }
            [chats_area, timeline_area, detail_area] => {
                self.chats.render(f, *chats_area, self.pane == Pane::Chats);
                self.timeline.render(
                    f,
                    *timeline_area,
                    &mut self.store,
                    self.pane == Pane::Timeline,
                );
                self.detail
                    .render(f, *detail_area, &mut self.store, self.pane == Pane::Detail);
            }
            _ => {}
        }
    }

    fn render_error(&self, f: &mut Frame, area: Rect, message: &str) {
        f.render_widget(
            Line::from(Span::styled(
                format!(" error: {message}"),
                Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            area,
        );
    }

    fn render_status(&self, f: &mut Frame, area: Rect) {
        let label = match self.mode() {
            Mode::Query => " query ",
            Mode::Help => " help ",
            Mode::Panes(Pane::Chats) => " chats ",
            Mode::Panes(Pane::Timeline) => " timeline ",
            Mode::Panes(Pane::Detail) => match self.detail.layout {
                DetailLayout::Full => " detail fullscreen ",
                DetailLayout::Split => " detail ",
            },
        };
        let nav: &str = match self.mode() {
            Mode::Query => "type a query  Enter/Tab keep it  Esc puts back  Ctrl+C quit",
            Mode::Help => "Esc / Enter / q / ? close this",
            Mode::Panes(Pane::Chats) => "j/k chats  Enter filter by chat",
            Mode::Panes(Pane::Timeline) => "j/k rows  Enter expand/open  g/G ends  f follow",
            Mode::Panes(Pane::Detail) => "j/k scroll  Enter all fields  f fullscreen",
        };
        f.render_widget(
            line![
                label.black().on_cyan(),
                "  ",
                nav.dark_gray(),
                "  Tab/h/l panes  / query  ? help  q quit"
            ],
            area,
        );
    }

    fn render_help(&self, f: &mut Frame, area: Rect) {
        let popup = centered_rect(72, 19, area);
        let text = Text::from(
            [
                "panes    Tab/h/l   cycle  |  / opens the query line",
                "query    type      chat:4242  kind:turn-ended  tool:send_*",
                "                   turn:5  slow:2s  err  \"free text\"",
                "         Enter/Tab keep the terms and close  |  Esc puts back the",
                "                   terms that were in effect when it opened",
                "         (chat:/kind: re-read the window as they change; the rest",
                "                   refilter what is already loaded)",
                "chats    j/k       move  |  Enter filter by that chat (again to clear)",
                "timeline j/k       move  |  Enter expand a turn, open an event",
                "         PgUp/PgDn, Ctrl+U/Ctrl+D  page  |  g/G oldest loaded",
                "                   (again: load older) / newest and follow  |  f follow",
                "detail   j/k       scroll  |  Enter every long field, again to fold",
                "         PgUp/PgDn, Ctrl+U/Ctrl+D  scroll  |  f fullscreen  |  Esc close",
                "global   ? help  q quit  Ctrl+C quit  Esc cancel / close",
            ]
            .join("\n"),
        );
        f.render_widget(
            Paragraph::new(text).block(Block::default().title(" help ").borders(Borders::ALL)),
            popup,
        );
    }
}

pub async fn run_tui(
    repos: Repos,
    chat_filter: Option<ChatId>,
    kind_filter: Option<EventKind>,
) -> Result<()> {
    App::new(repos, chat_filter, kind_filter).run().await
}

/// The query line as the command line spelled it, so the reader sees what `--chat`/`--event` chose.
fn seed_query(chat: Option<ChatId>, kind: Option<EventKind>) -> String {
    let mut terms: Vec<String> = Vec::new();
    if let Some(chat) = chat {
        terms.push(format!("chat:{}", chat.0));
    }
    if let Some(kind) = kind {
        terms.push(format!("kind:{}", kind.as_str()));
    }
    terms.join(" ")
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup[1])[1]
}
