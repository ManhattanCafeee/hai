use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use super::event_store::TurnGroup;

/// The chats the loaded window holds, with the turns each has in it. Counted here rather than queried:
/// the window is already in memory, so a chat with nothing in it is simply not listed.
pub(super) struct Chats {
    entries: Vec<Entry>,
    selected: usize,
    /// The list's height, from the last frame: what a page moves by.
    vp_h: usize,
}

struct Entry {
    chat_id: i64,
    turns: usize,
    /// Whether this is the chat the query narrows to — the one the pane marks.
    filtered: bool,
}

impl Chats {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            selected: 0,
            vp_h: 20,
        }
    }

    /// Recount from the turns the timeline kept, so a count always matches the rows that chat shows.
    pub fn rebuild(&mut self, groups: &[TurnGroup], filtered: Option<i64>) {
        let mut entries: Vec<Entry> = Vec::new();
        for group in groups {
            match entries
                .iter_mut()
                .find(|entry| entry.chat_id == group.chat_id)
            {
                Some(entry) => entry.turns += 1,
                None => entries.push(Entry {
                    chat_id: group.chat_id,
                    turns: 1,
                    filtered: filtered == Some(group.chat_id),
                }),
            }
        }
        self.entries = entries;
        if self.selected >= self.entries.len() {
            self.selected = self.entries.len().saturating_sub(1);
        }
    }

    pub fn selected_chat(&self) -> Option<i64> {
        self.entries.get(self.selected).map(|entry| entry.chat_id)
    }

    pub fn select_rel(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    /// Move the selection by half the pane's visible height, or by a quarter of it.
    pub fn page(&mut self, dir: isize, half: bool) {
        let step = (self.vp_h / if half { 4 } else { 2 }).max(1) as isize;
        self.select_rel(dir * step);
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, active: bool) {
        let border = if active {
            Color::White
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .title(" chats ")
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border));
        // The list keeps the selection in view by itself; this is only what a page moves by.
        self.vp_h = block.inner(area).height as usize;

        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| {
                let style = if entry.filtered {
                    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(Color::Gray)
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{:+}  {} turns", entry.chat_id, entry.turns),
                    style,
                )))
            })
            .collect();

        // The list keeps the selection in view by itself, so this pane holds no scroll state.
        let mut state = ListState::default().with_selected(Some(self.selected));
        f.render_stateful_widget(
            List::new(items).block(block).highlight_style(
                Style::new()
                    .fg(Color::Black)
                    .bg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD),
            ),
            area,
            &mut state,
        );
    }
}
