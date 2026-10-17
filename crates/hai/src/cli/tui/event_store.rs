use std::collections::{HashMap, HashSet};

use crate::{
    cli::display::{EventDisplay, TurnOutcome},
    domain::{
        model::Event,
        repo::Repos,
        vo::{ChatId, EventKind, TurnNumber},
    },
};

const PAGE_SIZE: usize = 200;

/// One turn's events in the order they happened: the unit a reader thinks in, while the log stores
/// events. A record that carries no turn of its own (a wrap-up) is a group of one.
pub(super) struct TurnGroup {
    pub chat_id: i64,
    pub turn: Option<TurnNumber>,
    pub seqs: Vec<i64>,
    /// What an open turn is remembered by. A turn's number restarts with each chapter, so `(chat,
    /// turn)` names several turns over a log's life; the turn's first record is in the window and
    /// names exactly one. A group the window begins inside is remembered by the first event of it the
    /// window holds, which is all this log can offer.
    pub anchor: i64,
    /// The turn's own start record, when the window holds it: what a row summarises the turn from.
    pub start_seq: Option<i64>,
    /// Present once the turn's own end record is in the window.
    pub outcome: Option<TurnOutcome>,
}

/// Groups the window into turns: a group runs while the chat id and the turn number stay the same, in
/// seq order. Grouping what the window holds, rather than what a filter keeps of it, is what lets a
/// turn stay one row when its start is filtered out.
pub(super) fn group_turns(rows: &[(i64, &EventDisplay)]) -> Vec<TurnGroup> {
    let mut groups: Vec<TurnGroup> = Vec::new();
    for (seq, display) in rows {
        let (chat_id, turn) = (display.chat_id, display.facts.turn);
        let continues = turn.is_some()
            && groups
                .last()
                .is_some_and(|group| group.chat_id == chat_id && group.turn == turn);
        if continues {
            let group = groups.last_mut().expect("a continuing group exists");
            group.seqs.push(*seq);
            if display.facts.outcome.is_some() {
                group.outcome = display.facts.outcome;
            }
        } else {
            groups.push(TurnGroup {
                chat_id,
                turn,
                seqs: vec![*seq],
                anchor: *seq,
                start_seq: (display.kind == Some(EventKind::TurnStarted)).then_some(*seq),
                outcome: display.facts.outcome,
            });
        }
    }
    groups
}

/// Seq-ascending sliding window over the events matching `chat_filter` and
/// `kind_filter`. Query errors propagate as `Result`; the `at_start` edge is
/// decided by what a query returned (a short or empty page), so a transient
/// error never poisons it and the load can be retried.
pub(super) struct EventStore {
    repos: Repos,
    pub chat_filter: Option<ChatId>,
    pub kind_filter: Option<EventKind>,

    window: Vec<Event>,
    pub at_start: bool,

    capacity: usize,
    cache: HashMap<i64, EventDisplay>,
}

impl EventStore {
    pub fn new(repos: Repos, chat_filter: Option<ChatId>, kind_filter: Option<EventKind>) -> Self {
        Self {
            repos,
            chat_filter,
            kind_filter,
            window: Vec::new(),
            at_start: false,
            capacity: PAGE_SIZE * 2,
            cache: HashMap::new(),
        }
    }

    pub fn set_viewport(&mut self, lines: usize) {
        self.capacity = (lines * 3).max(PAGE_SIZE * 2);
    }

    pub fn window(&self) -> &[Event] {
        &self.window
    }

    /// The window's displays, paired with their seqs, oldest first: what a pane groups and filters
    /// over. Every display is parsed at most once and read from the cache, so a rebuild costs no
    /// payload parse.
    pub fn displays(&mut self) -> Vec<(i64, &EventDisplay)> {
        let seqs: Vec<i64> = self.window.iter().map(|event| event.seq).collect();
        for &seq in &seqs {
            // Fills the cache; the rows are read back below, once every display is in place.
            let _ = self.display_by_seq(seq);
        }
        seqs.iter()
            .filter_map(|&seq| self.cache.get(&seq).map(|display| (seq, display)))
            .collect()
    }

    /// The window is seq-ascending, so a lookup is a binary search rather than a scan: a frame asks for
    /// every row it shows.
    pub fn event_by_seq(&self, seq: i64) -> Option<&Event> {
        let index = self
            .window
            .binary_search_by_key(&seq, |event| event.seq)
            .ok()?;
        self.window.get(index)
    }

    /// Cached display for one seq, so an event is deserialized at most once. Borrowed rather than
    /// cloned: a `TurnStarted`'s detail runs to tens of kilobytes, and every redraw reads it.
    pub fn display_by_seq(&mut self, seq: i64) -> Option<&EventDisplay> {
        if !self.cache.contains_key(&seq) {
            let event = self.event_by_seq(seq)?;
            let display = EventDisplay::from_event(event).unwrap_or_else(EventDisplay::unparsed);
            self.cache.insert(seq, display);
        }
        self.cache.get(&seq)
    }

    pub fn min_seq(&self) -> Option<i64> {
        self.window.first().map(|e| e.seq)
    }

    pub fn max_seq(&self) -> Option<i64> {
        self.window.last().map(|e| e.seq)
    }

    pub async fn load_end(&mut self) -> crate::error::Result<()> {
        let mut events = self
            .repos
            .event
            .query(
                self.chat_filter,
                self.kind_filter,
                None,
                None,
                true,
                self.capacity,
            )
            .await?;
        events.reverse(); // query returns newest-first; the window is seq-ascending
        // A short page means nothing older exists.
        self.at_start = events.len() < self.capacity;
        self.set_window(events);
        Ok(())
    }

    pub async fn extend_back(&mut self) -> crate::error::Result<usize> {
        let Some(min) = self.min_seq() else {
            self.load_end().await?;
            return Ok(0);
        };
        let mut older = self
            .repos
            .event
            .query(
                self.chat_filter,
                self.kind_filter,
                Some(min),
                None,
                true,
                self.capacity,
            )
            .await?;
        older.reverse();
        if older.is_empty() {
            self.at_start = true;
            return Ok(0);
        }
        self.at_start = false;
        let n = older.len();
        self.window.splice(0..0, older);
        Ok(n)
    }

    pub async fn append_new(&mut self) -> crate::error::Result<usize> {
        let after = self.max_seq();
        let events = self
            .repos
            .event
            .query(
                self.chat_filter,
                self.kind_filter,
                None,
                after,
                false,
                self.capacity,
            )
            .await?;
        if events.is_empty() {
            return Ok(0);
        }
        let n = events.len();
        self.window.extend(events);
        Ok(n)
    }

    pub async fn count_new(&mut self) -> crate::error::Result<usize> {
        let Some(after) = self.max_seq() else {
            return Ok(0);
        };
        let count = self
            .repos
            .event
            .count_after(after, self.chat_filter, self.kind_filter)
            .await?;
        Ok(count.max(0) as usize)
    }

    fn set_window(&mut self, events: Vec<Event>) {
        self.window = events;
        self.trim_to_capacity_back();
    }

    // Only a freshly loaded window is trimmed; prepends and appends keep every
    // loaded event so browsed history and the selection stay addressable.
    fn trim_to_capacity_back(&mut self) {
        if self.window.len() > self.capacity {
            self.window.truncate(self.capacity);
        }
        self.trim_cache();
    }

    /// Drops entries that left the window once the cache outgrows it.
    fn trim_cache(&mut self) {
        if self.cache.len() > self.capacity * 4 {
            let window: HashSet<i64> = self.window.iter().map(|event| event.seq).collect();
            self.cache.retain(|seq, _| window.contains(seq));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::display::EventFacts;

    fn display(
        kind: EventKind,
        chat_id: i64,
        turn: Option<usize>,
        outcome: Option<TurnOutcome>,
    ) -> EventDisplay {
        EventDisplay {
            tag: "T",
            one_liner: String::new(),
            sections: Vec::new(),
            chat_id,
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

    fn successful(tools: usize) -> TurnOutcome {
        TurnOutcome {
            tools: Some(tools),
            elapsed_ms: Some(1200),
            failed: false,
            replied: true,
        }
    }

    /// A turn is one group; a chat switch, a new turn number, or a record carrying no turn starts a
    /// new one — and the group ends up with the outcome its end record reported and the start its own
    /// record gave it.
    #[test]
    fn turns_group_by_chat_and_turn() {
        let (first, second) = (
            display(EventKind::TurnStarted, 1, Some(1), None),
            display(EventKind::TurnEnded, 1, Some(1), Some(successful(2))),
        );
        let (other_chat, next_turn) = (
            display(EventKind::TurnStarted, 2, Some(1), None),
            display(EventKind::TurnStarted, 1, Some(2), None),
        );
        let (wrap_up, wrap_up_again) = (
            display(EventKind::WrapUpStarted, 1, None, None),
            display(EventKind::WrapUpStarted, 1, None, None),
        );
        let rows = [
            (10, &first),
            (11, &second),
            (12, &other_chat),
            (13, &next_turn),
            (14, &wrap_up),
            (15, &wrap_up_again),
        ];

        let groups = group_turns(&rows);

        assert_eq!(groups.len(), 5, "one per turn, chat, or turn-less record");
        assert_eq!(groups[0].seqs, [10, 11]);
        assert_eq!(groups[0].turn, Some(TurnNumber(1)));
        assert_eq!(groups[0].outcome.expect("the end record").tools, Some(2));
        assert_eq!(groups[0].start_seq, Some(10), "the turn's own start");
        assert_eq!(groups[1].chat_id, 2, "another chat is another group");
        assert_eq!(groups[3].turn, None);
        assert_eq!(
            groups[3].start_seq, None,
            "a record with no turn has no start"
        );
        assert_eq!(groups[4].seqs, [15], "a turn-less record stands alone");
    }
}
