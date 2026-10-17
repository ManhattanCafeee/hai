use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};

use super::event_store::EventStore;
use crate::cli::display::{self, EventDisplay};

/// How far the detail's paged moves go; they are line counts, not the pane's height.
const PAGE_LINES: isize = 10;
const HALF_PAGE_LINES: isize = 15;

/// Whether the detail shares the body with the timeline or takes all of it.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum DetailLayout {
    Split,
    Full,
}

/// One event in full: the sections fold to their preview until the reader asks for all of them.
pub(super) struct Detail {
    seq: Option<i64>,
    scroll: u16,
    unfolded: bool,
    width: usize,
    pub layout: DetailLayout,
}

impl Detail {
    pub fn new() -> Self {
        Self {
            seq: None,
            scroll: 0,
            unfolded: false,
            width: 1,
            layout: DetailLayout::Split,
        }
    }

    pub fn is_open(&self) -> bool {
        self.seq.is_some()
    }

    /// Point the pane at an event; another event starts its scroll and its folds afresh.
    pub fn show(&mut self, seq: Option<i64>) {
        if self.seq != seq {
            self.scroll = 0;
            self.unfolded = false;
        }
        self.seq = seq;
    }

    pub fn close(&mut self) {
        self.seq = None;
        self.scroll = 0;
    }

    /// A pane whose event left the window has nothing to show and closes.
    pub fn keep_in_window(&mut self, store: &EventStore) {
        if let Some(seq) = self.seq
            && store.event_by_seq(seq).is_none()
        {
            self.close();
        }
    }

    pub fn toggle_unfold(&mut self) {
        self.unfolded = !self.unfolded;
    }

    pub fn toggle_layout(&mut self) {
        self.layout = match self.layout {
            DetailLayout::Split => DetailLayout::Full,
            DetailLayout::Full => DetailLayout::Split,
        };
    }

    pub fn scroll(&mut self, delta: isize, store: &mut EventStore) {
        let max = self
            .body_of(store)
            .map_or(0, |text| self.wrapped_lines(&text).saturating_sub(1));
        self.scroll = (self.scroll as isize + delta).clamp(0, max as isize) as u16;
    }

    /// Scroll by a page, or by half of one: the detail counts its pages in lines rather than in the
    /// pane's height, since a field's worth of text is what the reader is moving through.
    pub fn page(&mut self, dir: isize, half: bool, store: &mut EventStore) {
        self.scroll(dir * if half { HALF_PAGE_LINES } else { PAGE_LINES }, store);
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, store: &mut EventStore, active: bool) {
        let border = if active {
            Color::White
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .title(self.pane_title(store))
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border));

        let inner = block.inner(area);
        self.width = inner.width as usize;
        let text = self.body_of(store).unwrap_or_default();
        let scroll_len = self.wrapped_lines(&text) as usize + 1;
        self.scroll = (self.scroll as usize).min(scroll_len.saturating_sub(1)) as u16;

        f.render_widget(
            Paragraph::new(text)
                .block(block)
                .scroll((self.scroll, 0))
                .wrap(Wrap { trim: false }),
            area,
        );

        let pos = self.scroll as usize;
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            inner,
            &mut ScrollbarState::new(scroll_len).position(pos),
        );
    }

    /// The header facts as the pane's title, so the reader always knows which event is on the right.
    fn pane_title(&self, store: &mut EventStore) -> String {
        let Some(seq) = self.seq else {
            return String::from(" detail ");
        };
        let Some(time) = store
            .event_by_seq(seq)
            .map(|event| display::fmt_time(event.created_at))
        else {
            return String::from(" detail ");
        };
        match store.display_by_seq(seq) {
            Some(display) => format!(" #{seq}  {time}  {:+}  {} ", display.chat_id, display.tag),
            None => String::from(" detail "),
        }
    }

    /// The body: each section's title, the lines it shows, and the marker that stands in for the lines
    /// a fold hides.
    fn body(&self, display: &EventDisplay) -> Vec<Line<'static>> {
        let folded = !self.unfolded;
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (index, section) in display.sections.iter().enumerate() {
            if index > 0 {
                lines.push(Line::raw(""));
            }
            let marker = match (section.long, folded) {
                (false, _) => "  ",
                (true, true) => "▸ ",
                (true, false) => "▾ ",
            };
            lines.push(Line::from(vec![
                Span::styled(marker, Style::new().fg(Color::DarkGray)),
                Span::styled(
                    section.title,
                    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::raw(":"),
            ]));
            for line in section.shown(folded) {
                lines.push(Line::raw(line.to_string()));
            }
            if let Some(note) = section.fold_note(folded) {
                lines.push(Line::styled(note, Style::new().fg(Color::DarkGray)));
            }
        }
        lines
    }

    fn body_of(&self, store: &mut EventStore) -> Option<Text<'static>> {
        let seq = self.seq?;
        let display = store.display_by_seq(seq)?;
        Some(Text::from(self.body(display)))
    }

    /// How many lines the body takes once wrapped: what the scroll bound and the paragraph agree on.
    fn wrapped_lines(&self, text: &Text) -> u16 {
        let width = self.width.max(1);
        text.lines
            .iter()
            .map(|line| line.width().div_ceil(width))
            .sum::<usize>()
            .min(u16::MAX as usize) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::display::{EventFacts, PREVIEW_LINES, Section};

    fn display(sections: Vec<Section>) -> EventDisplay {
        EventDisplay {
            tag: "TURN",
            one_liner: String::new(),
            sections,
            chat_id: 7,
            kind: None,
            color: (0, 0, 0),
            facts: EventFacts {
                turn: None,
                tool: None,
                ok: None,
                outcome: None,
            },
        }
    }

    fn plain(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn body(detail: &Detail, display: &EventDisplay) -> Vec<String> {
        detail.body(display).iter().map(plain).collect()
    }

    /// Folding shows a section's preview and says how much it hides; unfolding shows it whole. Only a
    /// section long enough to bury the rest gets a marker.
    #[test]
    fn a_long_section_folds_its_body() {
        let long = Section {
            title: "Prompt",
            lines: (1..=20).map(|n| format!("  line {n}")).collect(),
            long: true,
        };
        let short = Section {
            title: "Fields",
            lines: vec![String::from("  Turn:  5")],
            long: false,
        };
        let display = display(vec![short, long]);

        let folded = Detail::new();
        let text = body(&folded, &display);
        assert_eq!(text[0], "  Fields:");
        assert_eq!(text[1], "  Turn:  5");
        assert_eq!(text[2], "", "sections are set apart by a blank line");
        assert_eq!(text[3], "▸ Prompt:");
        assert_eq!(text[4], "  line 1");
        assert_eq!(
            text[4 + PREVIEW_LINES],
            "… +14 more lines (Enter to expand)"
        );

        let mut unfolded = Detail::new();
        unfolded.toggle_unfold();
        let text = body(&unfolded, &display);
        assert_eq!(text[3], "▾ Prompt:");
        assert_eq!(text.len(), 4 + 20, "the whole body and no marker line left");
    }

    /// Another event starts the pane's scroll and folds over; the same event keeps them.
    #[test]
    fn the_pane_keeps_its_state_per_event() {
        let mut detail = Detail::new();
        detail.show(Some(5));
        detail.toggle_unfold();
        assert!(detail.is_open());
        assert!(detail.unfolded);

        detail.show(Some(5));
        assert!(detail.unfolded, "the same event keeps its folds");

        detail.show(Some(6));
        assert!(!detail.unfolded, "another event starts fresh");
        assert_eq!(detail.scroll, 0);

        detail.close();
        assert!(!detail.is_open());
    }
}
