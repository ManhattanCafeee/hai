//! The event log timeline: the turn groups, the rows inside them, and where the selection sits.

use std::collections::{HashMap, HashSet};

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use super::{
    event_store::{EventStore, TurnGroup, group_turns},
    filter::Terms,
};
use crate::cli::display::{self, EventDisplay};

const FAILED: (u8, u8, u8) = (239, 68, 68);

/// What the selection points at: an event's seq and whether the row is a child of an open turn. An
/// open turn's first event is both its header and one of its children, so the seq alone does not name
/// a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct RowKey {
    pub seq: i64,
    pub child: bool,
}

/// One row of the timeline: the header of a turn, or one of its events while the turn is open.
#[derive(Debug, Clone, Copy)]
struct Row {
    key: RowKey,
    /// Index into `Timeline::groups`.
    group: usize,
}

/// The window's turns, and the rows the viewport shows of them.
pub(super) struct Timeline {
    groups: Vec<TurnGroup>,
    rows: Vec<Row>,
    /// The open turns, keyed by the first record of each (`TurnGroup::anchor`): a turn number restarts
    /// with every chapter, so it is not an identity a log can be read by.
    expanded: HashSet<i64>,
    pub selected: Option<RowKey>,
    pub follow: bool,
    pub new_count: usize,
    offset: usize,
    vp_h: usize,
    pub back_loading: bool,
}

impl Timeline {
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            rows: Vec::new(),
            expanded: HashSet::new(),
            selected: None,
            follow: false,
            new_count: 0,
            offset: 0,
            vp_h: 20,
            back_loading: false,
        }
    }

    /// Regroup the window with the query applied. Rows the reader had open stay open, and a selection
    /// whose row is still there stays put.
    pub fn rebuild(&mut self, displays: &[(i64, &EventDisplay)], query: &Terms) {
        let (groups, rows) = rows_for(displays, &self.expanded, query);
        self.groups = groups;
        self.rows = rows;
        if let Some(key) = self.selected {
            self.selected = self.reselect(key);
        }
        self.clamp_offset();
    }

    /// Where the selection goes after a rebuild. A prepend can change which event of a turn comes
    /// first, so the row the reader was on can be gone while the turn it stood for is still there: the
    /// selection follows its event to that turn's row rather than falling back to the head, which would
    /// leave the reader at the top of every freshly loaded page.
    fn reselect(&self, key: RowKey) -> Option<RowKey> {
        if let Some(row) = self.rows.iter().find(|row| row.key == key) {
            return Some(row.key);
        }
        self.rows
            .iter()
            .find(|row| !row.key.child && self.groups[row.group].seqs.contains(&key.seq))
            .map(|row| row.key)
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The turns the window holds, after the query: what the chats pane counts.
    pub fn groups(&self) -> &[TurnGroup] {
        &self.groups
    }

    pub fn vp_h(&self) -> usize {
        self.vp_h
    }

    pub fn selected_seq(&self) -> Option<i64> {
        self.selected.map(|key| key.seq)
    }

    pub fn selected_row(&self) -> Option<RowKey> {
        self.selected
    }

    /// Where the selection sits: the tail while following, the head otherwise.
    pub fn selected_index(&self) -> usize {
        let fallback = if self.follow {
            self.rows.len().saturating_sub(1)
        } else {
            0
        };
        self.selected
            .and_then(|key| self.index_of(key))
            .unwrap_or(fallback)
    }

    pub fn select_at(&mut self, index: usize) {
        if self.rows.is_empty() {
            self.selected = None;
            return;
        }
        let index = index.clamp(0, self.rows.len() - 1);
        self.selected = Some(self.rows[index].key);
        self.follow = index + 1 == self.rows.len();
        self.clamp_offset();
    }

    pub fn select_rel(&mut self, delta: isize) {
        let index = (self.selected_index() as isize + delta).max(0) as usize;
        self.select_at(index);
    }

    /// Move the selection by half the pane's visible height, or by a quarter of it.
    pub fn page(&mut self, dir: isize, half: bool) {
        let step = (self.vp_h / if half { 4 } else { 2 }).max(1) as isize;
        self.select_rel(dir * step);
    }

    /// Put the selection on the newest turn's header and keep following. A record that carries no turn
    /// of its own never takes the selection: a stream of wrap-ups cannot pull the reader off the last
    /// thing that happened.
    pub fn follow_end(&mut self) {
        self.follow = true;
        self.new_count = 0;
        let newest_turn = self
            .rows
            .iter()
            .rposition(|row| !row.key.child && self.groups[row.group].turn.is_some());
        self.selected = newest_turn
            .or(self.rows.len().checked_sub(1))
            .map(|index| self.rows[index].key);
        self.clamp_offset();
    }

    /// Open the turn a row belongs to, or close it. A record that carries no turn of its own stands
    /// alone and has nothing to open, so nothing happens — `rebuild` turns the new state into rows.
    pub fn toggle(&mut self, key: RowKey) {
        let Some(row) = self.rows.iter().find(|row| row.key == key) else {
            return;
        };
        let group = &self.groups[row.group];
        if group.turn.is_none() {
            return;
        }
        let anchor = group.anchor;
        if self.expanded.contains(&anchor) {
            self.expanded.remove(&anchor);
        } else {
            self.expanded.insert(anchor);
        }
    }

    /// The group a row belongs to.
    pub fn group_of(&self, key: RowKey) -> Option<&TurnGroup> {
        let row = self.rows.iter().find(|row| row.key == key)?;
        self.groups.get(row.group)
    }

    pub fn clamp_offset(&mut self) {
        if self.rows.is_empty() || self.vp_h == 0 {
            return;
        }
        let max_off = self.rows.len().saturating_sub(self.vp_h);
        let scrolloff = (self.vp_h / 3).max(1);
        let selected = self.selected_index();
        self.offset = selected.saturating_sub(scrolloff).min(max_off);
    }

    /// Re-anchor the view after a prepend: `visual` is the selected row's distance from the viewport
    /// top before the load.
    pub fn anchor_after_prepend(&mut self, visual: usize) {
        if self.rows.is_empty() || self.vp_h == 0 {
            return;
        }
        let max_off = self.rows.len().saturating_sub(self.vp_h);
        let selected = self.selected_index();
        self.offset = selected.saturating_sub(visual).min(max_off);
    }

    pub fn visual_pos(&self) -> usize {
        self.selected_index().saturating_sub(self.offset)
    }

    fn index_of(&self, key: RowKey) -> Option<usize> {
        self.rows.iter().position(|row| row.key == key)
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, store: &mut EventStore, active: bool) {
        let border = if active {
            Color::White
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .title(" timeline ")
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border));

        let height = block.inner(area).height as usize;
        if height != self.vp_h {
            // The viewport changed — the first frame after the placeholder height, or a resize — so
            // the offset is recomputed around the selection instead of being left over from the old
            // height. A steady frame leaves it alone, which is what keeps a prepend's anchor.
            self.vp_h = height;
            self.clamp_offset();
        }
        let max_off = self.rows.len().saturating_sub(self.vp_h);
        self.offset = self.offset.min(max_off);

        // Only the rows the viewport shows become widgets: the window holds thousands of events, and
        // one row's summary is built from a payload that may run to tens of kilobytes.
        let end = (self.offset + self.vp_h).min(self.rows.len());
        let mut items = Vec::with_capacity(end - self.offset);
        for row in &self.rows[self.offset..end] {
            let Some(item) = self.row_item(row, store) else {
                continue;
            };
            items.push(item);
        }

        let mut state = ListState::default()
            .with_offset(0)
            .with_selected(Some(self.selected_index().saturating_sub(self.offset)));

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

    fn row_item(&self, row: &Row, store: &mut EventStore) -> Option<ListItem<'static>> {
        let seq = row.key.seq;
        let time = display::fmt_time(store.event_by_seq(seq)?.created_at);
        let group = self.groups.get(row.group)?;
        // A filter can hide a turn's start row from the rows, so its line is read by seq.
        let summary = group
            .start_seq
            .filter(|start| *start != seq)
            .and_then(|start| store.display_by_seq(start))
            .map(|start| start.one_liner.clone());
        let display = store.display_by_seq(seq)?;
        let text = if row.key.child {
            format!(
                "    #{seq:>5}  {time}  {:+}  {}",
                display.chat_id, display.one_liner
            )
        } else {
            header_line(
                group,
                seq,
                &time,
                display,
                summary.as_deref(),
                &self.expanded,
            )
        };
        let (cr, cg, cb) = if !row.key.child && group.outcome.is_some_and(|outcome| outcome.failed)
        {
            FAILED
        } else {
            display.color
        };
        Some(ListItem::new(Line::from(Span::styled(
            text,
            Style::new().fg(Color::Rgb(cr, cg, cb)),
        ))))
    }
}

/// The window's displays as turn groups, and the rows they show: one header per group the query keeps
/// and, while a turn is open, one row per matching event. The query decides which events of a turn
/// are listed and whether the turn is listed at all — never what the turn is summarised as.
fn rows_for(
    displays: &[(i64, &EventDisplay)],
    expanded: &HashSet<i64>,
    query: &Terms,
) -> (Vec<TurnGroup>, Vec<Row>) {
    let by_seq: HashMap<i64, &EventDisplay> = displays.iter().copied().collect();

    let mut groups: Vec<TurnGroup> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    for mut group in group_turns(displays) {
        group.seqs.retain(|seq| {
            by_seq
                .get(seq)
                .is_some_and(|display| query.matches(*seq, display))
        });
        if group.seqs.is_empty() {
            continue;
        }
        let index = groups.len();
        let open = expanded.contains(&group.anchor);
        rows.push(Row {
            key: RowKey {
                seq: group.seqs[0],
                child: false,
            },
            group: index,
        });
        if open {
            rows.extend(group.seqs.iter().map(|seq| Row {
                key: RowKey {
                    seq: *seq,
                    child: true,
                },
                group: index,
            }));
        }
        groups.push(group);
    }
    (groups, rows)
}

/// One turn as one line: the marker that says whether it is open, what the turn was, and how it
/// ended. `summary` is the turn's own start line when that event is not the row itself.
fn header_line(
    group: &TurnGroup,
    seq: i64,
    time: &str,
    display: &EventDisplay,
    summary: Option<&str>,
    expanded: &HashSet<i64>,
) -> String {
    let Some(turn) = group.turn else {
        // A record that carries no turn of its own stands alone: its rendering is the whole row.
        return format!(
            "#{seq:>5}  {time}  {:+}  {}",
            display.chat_id, display.one_liner
        );
    };
    let marker = if expanded.contains(&group.anchor) {
        "▾"
    } else {
        "▸"
    };
    let mut line = format!("#{seq:>5}  {time}  {:+}  {marker}  ", display.chat_id);
    match (summary, group.start_seq) {
        (Some(summary), _) => line.push_str(summary),
        (None, Some(_)) => line.push_str(&display.one_liner),
        // The window begins inside the turn: say so rather than pass a child off as the whole turn.
        (None, None) => line.push_str(&format!("… Turn {turn}  (older events not loaded)")),
    }
    match group.outcome {
        Some(outcome) => {
            let mark = if outcome.failed {
                "✗"
            } else if outcome.replied {
                "✓"
            } else {
                "–"
            };
            line.push_str("  →");
            if let (Some(tools), Some(ms)) = (outcome.tools, outcome.elapsed_ms) {
                line.push_str(&format!("  {tools} tools  {:.1}s", ms as f64 / 1000.0));
            }
            line.push_str(&format!("  {mark}"));
        }
        // Only a turn whose start is in the window can be said to be running: another turn's end may
        // simply be further along than the window reaches.
        None if group.start_seq.is_some() => line.push_str("  running"),
        None => {}
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::display::{EventFacts, TurnOutcome},
        domain::vo::{EventKind, TurnNumber},
    };

    fn display(kind: EventKind, turn: Option<usize>, outcome: Option<TurnOutcome>) -> EventDisplay {
        EventDisplay {
            tag: "T",
            one_liner: String::from("TURN  5  direct  msgs:3  by:window"),
            sections: Vec::new(),
            chat_id: 7,
            kind: Some(kind),
            color: (0, 0, 0),
            facts: EventFacts {
                turn: turn.map(TurnNumber),
                tool: None,
                ok: None,
                outcome,
            },
        }
    }

    fn ended() -> TurnOutcome {
        TurnOutcome {
            tools: Some(1),
            elapsed_ms: Some(1200),
            failed: false,
            replied: true,
        }
    }

    /// Collapsed, a turn is one row; open, it lists the events that happened inside it.
    #[test]
    fn a_collapsed_turn_is_one_row_and_an_expanded_one_lists_its_events() {
        let (first, second) = (
            display(EventKind::TurnStarted, Some(5), None),
            display(EventKind::TurnEnded, Some(5), Some(ended())),
        );
        let displays = [(10, &first), (11, &second)];

        let (groups, rows) = rows_for(&displays, &HashSet::new(), &Terms::default());
        assert_eq!(groups.len(), 1, "one turn is one group");
        assert_eq!(groups[0].outcome.expect("the end record").tools, Some(1));
        assert_eq!(rows.len(), 1, "collapsed, the turn is one row");
        assert_eq!(
            rows[0].key.seq, 10,
            "the header anchors on the turn's start"
        );

        let mut expanded = HashSet::new();
        expanded.insert(10);
        let (_, rows) = rows_for(&displays, &expanded, &Terms::default());
        assert_eq!(rows.len(), 3, "the header plus its two events");
        assert_eq!(
            rows[1].key,
            RowKey {
                seq: 10,
                child: true
            }
        );
        assert_eq!(
            rows[2].key,
            RowKey {
                seq: 11,
                child: true
            }
        );
    }

    /// A query that keeps any of a turn's events keeps the turn, showing only what matched; a query
    /// that keeps none of them drops the turn whole.
    #[test]
    fn a_filtered_turn_loses_its_rows() {
        let (first, second) = (
            display(EventKind::TurnStarted, Some(5), None),
            display(EventKind::TurnEnded, Some(5), Some(ended())),
        );
        let displays = [(10, &first), (11, &second)];
        let mut expanded = HashSet::new();
        expanded.insert(10);

        let keep_second = Terms::parse("kind:turn-ended").expect("a query that parses");
        let (groups, rows) = rows_for(&displays, &expanded, &keep_second);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].seqs, [11], "only the matching event is left");
        assert_eq!(rows.len(), 2, "the header, plus the one event that matched");
        assert_eq!(rows[0].key.seq, 11, "the header anchors on what is left");
        assert_eq!(
            rows[1].key,
            RowKey {
                seq: 11,
                child: true
            },
            "only the matching child is listed"
        );

        let keep_nothing = Terms::parse("kind:tool-call").expect("a query that parses");
        let (groups, rows) = rows_for(&displays, &expanded, &keep_nothing);
        assert!(
            groups.is_empty(),
            "a turn none of whose events match is gone"
        );
        assert!(rows.is_empty());
    }

    /// Following puts the selection on the newest turn's header, not on a trailing record that carries
    /// no turn, and only a turn opens.
    #[test]
    fn following_lands_on_the_newest_turn() {
        let (turn, wrap_up) = (
            display(EventKind::TurnStarted, Some(5), Some(ended())),
            display(EventKind::WrapUpStarted, None, None),
        );
        let displays = [(10, &turn), (11, &wrap_up)];
        let (groups, rows) = rows_for(&displays, &HashSet::new(), &Terms::default());

        let mut timeline = Timeline::new();
        timeline.groups = groups;
        timeline.rows = rows;
        timeline.follow_end();
        assert_eq!(
            timeline.selected_seq(),
            Some(10),
            "the turn, not the record"
        );
        assert_eq!(timeline.rows.len(), 2, "the turn's header and the record");
        assert!(timeline.follow);

        let record = RowKey {
            seq: 11,
            child: false,
        };
        let header = RowKey {
            seq: 10,
            child: false,
        };

        timeline.toggle(record);
        timeline.rebuild(&displays, &Terms::default());
        assert_eq!(timeline.rows.len(), 2, "a record has nothing to open");

        timeline.toggle(header);
        timeline.rebuild(&displays, &Terms::default());
        assert_eq!(timeline.rows.len(), 3, "open, the turn shows its event");

        timeline.toggle(header);
        timeline.rebuild(&displays, &Terms::default());
        assert_eq!(timeline.rows.len(), 2, "and closes again");
        assert_eq!(timeline.selected_seq(), Some(10), "the selection holds");
        assert!(timeline.follow, "closing a turn is not moving the reader");
    }

    /// A page moves by the pane's visible height, a quarter of it for the half-page keys.
    #[test]
    fn paging_moves_by_the_pane() {
        let turns: Vec<EventDisplay> = (1..=40)
            .map(|turn| display(EventKind::TurnStarted, Some(turn), None))
            .collect();
        let displays: Vec<(i64, &EventDisplay)> = turns
            .iter()
            .enumerate()
            .map(|(index, display)| (index as i64 + 1, display))
            .collect();

        let mut timeline = Timeline::new();
        timeline.rebuild(&displays, &Terms::default());
        assert_eq!(
            timeline.vp_h(),
            20,
            "the placeholder height until a frame sets it"
        );

        timeline.page(1, false);
        assert_eq!(timeline.selected_index(), 10, "half the pane");
        timeline.page(1, true);
        assert_eq!(timeline.selected_index(), 15, "a quarter more");
        timeline.page(-1, false);
        assert_eq!(timeline.selected_index(), 5);
    }

    /// A prepend can hand a turn events that come before the one the reader was on. The selection
    /// follows its turn there; falling back to the head would re-anchor at the top and pull page
    /// after page.
    #[test]
    fn a_prepend_keeps_the_selection() {
        let (third, fourth) = (
            display(EventKind::ToolCallResult, Some(1), None),
            display(EventKind::StepCompleted, Some(1), None),
        );
        let window = [(4, &third), (5, &fourth)];
        let mut timeline = Timeline::new();
        timeline.rebuild(&window, &Terms::default());
        timeline.select_at(0);
        assert_eq!(
            timeline.selected_row().map(|key| key.seq),
            Some(4),
            "the turn's first event in this window"
        );

        // Older events arrive: the same turn now begins three events earlier.
        let (first, second) = (
            display(EventKind::TurnStarted, Some(1), None),
            display(EventKind::ToolCall, Some(1), None),
        );
        let grown = [(1, &first), (2, &second), (4, &third), (5, &fourth)];
        timeline.rebuild(&grown, &Terms::default());

        assert_eq!(
            timeline.selected_row(),
            Some(RowKey {
                seq: 1,
                child: false
            }),
            "the selection moved with its turn, not to the head"
        );
    }

    /// A turn's number restarts with each chapter, so one chat's log holds several turns numbered 5.
    /// Opening one has to leave the others shut: an open turn is remembered by its own first record,
    /// not by a number the log hands out again.
    #[test]
    fn opening_a_turn_leaves_another_chapter_shut() {
        let first = display(EventKind::TurnStarted, Some(5), Some(ended()));
        let wrap_up = display(EventKind::WrapUpStarted, None, None);
        let second = display(EventKind::TurnStarted, Some(5), Some(ended()));
        let displays = [(10, &first), (20, &wrap_up), (30, &second)];

        let mut timeline = Timeline::new();
        timeline.rebuild(&displays, &Terms::default());
        assert_eq!(
            timeline.rows.len(),
            3,
            "two turns of one number, and the wrap-up"
        );

        timeline.toggle(RowKey {
            seq: 10,
            child: false,
        });
        timeline.rebuild(&displays, &Terms::default());
        assert_eq!(timeline.rows.len(), 4, "the opened turn shows its event");
        assert_eq!(
            timeline.rows.iter().filter(|row| row.key.child).count(),
            1,
            "one turn was opened, so one child row exists"
        );
    }
}
