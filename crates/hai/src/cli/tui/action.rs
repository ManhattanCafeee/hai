use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// The panes, in cycling order: the views of the window. The query line is not one of them — it takes
/// the keyboard only while it is open, and gives it back when it closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pane {
    Chats,
    Timeline,
    Detail,
}

impl Pane {
    const ORDER: [Pane; 3] = [Pane::Chats, Pane::Timeline, Pane::Detail];

    /// `delta` steps along the cycle, stepping over the detail pane while it shows nothing.
    pub fn cycle(self, delta: isize, detail_open: bool) -> Self {
        let len = Self::ORDER.len() as isize;
        let from = Self::ORDER
            .iter()
            .position(|pane| *pane == self)
            .unwrap_or(0) as isize;
        let mut to = (from + delta).rem_euclid(len);
        if Self::ORDER[to as usize] == Pane::Detail && !detail_open {
            to = (to + delta.signum()).rem_euclid(len);
        }
        Self::ORDER[to as usize]
    }
}

/// What owns the keyboard: the query line and the help box take it from the panes while they are up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Panes(Pane),
    Query,
    Help,
}

/// What a key means. Reading a key apart from acting on it is what makes the keymap testable without
/// a terminal.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Action {
    Quit,
    ToggleHelp,
    CyclePane(isize),
    /// Take the keyboard for the query line, on the terms that are in effect.
    OpenQuery,
    /// Keep what was typed and give the keyboard back.
    CloseQuery,
    /// Give the keyboard back and put the terms in effect back the way they were.
    CancelQuery,
    /// Move the focused pane's cursor: a list selection, a timeline row, or the detail's scroll.
    Move(isize),
    /// Move it by half the pane's visible height, in the direction given (`PageUp`/`PageDown`).
    Page(isize),
    /// Move it by a quarter of that height (`Ctrl+U`/`Ctrl+D`).
    HalfPage(isize),
    /// Jump to the oldest loaded row; pressed there again, it loads one older page.
    JumpHead,
    /// Jump to the newest row and follow it.
    JumpTail,
    ToggleFollow,
    /// Give the detail the whole body, or give it back to the timeline.
    ToggleFullscreen,
    /// The focused pane's primary action: expand a turn, open an event's detail, unfold the detail.
    Act,
    /// Cancel: close the detail, or clear the error line; the help and the query line are modes of
    /// their own and handle their own `Esc`.
    Cancel,
    /// A key for the query editor to consume.
    QueryKey(KeyEvent),
}

/// The work an action asks for; the loop runs it against the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Effect {
    LoadEnd,
    LoadOlder,
    Reload,
}

/// The action a key means in the mode it was pressed in, or `None` for a key that means nothing there.
pub(super) fn key_event(key: KeyEvent, mode: Mode) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    let code = key.code;

    // Raw mode suppresses SIGINT, so Ctrl+C is the quit that works even mid-word.
    if key.modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    match mode {
        // The help owns the keys while it is up.
        Mode::Help => matches!(
            code,
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?')
        )
        .then_some(Action::ToggleHelp),
        // The query line owns them while it is open. `Tab` is a way out like `Enter`, since a term
        // applies as it is typed.
        Mode::Query => Some(match code {
            KeyCode::Esc => Action::CancelQuery,
            KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab => Action::CloseQuery,
            _ => Action::QueryKey(key),
        }),
        Mode::Panes(pane) => panes_key_event(pane, key, code),
    }
}

/// The keys the panes share, and the ones the focused pane keeps for itself.
fn panes_key_event(pane: Pane, key: KeyEvent, code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(Action::Quit),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Tab | KeyCode::Char('l') => Some(Action::CyclePane(1)),
        KeyCode::BackTab | KeyCode::Char('h') => Some(Action::CyclePane(-1)),
        KeyCode::Char('/') => Some(Action::OpenQuery),
        KeyCode::PageUp => Some(Action::Page(-1)),
        KeyCode::PageDown => Some(Action::Page(1)),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::HalfPage(-1))
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::HalfPage(1))
        }
        _ => pane_own_key(pane, code),
    }
}

/// The keys a pane keeps once the shared ones are out of the way.
fn pane_own_key(pane: Pane, code: KeyCode) -> Option<Action> {
    let rows = |code: KeyCode| match code {
        KeyCode::Char('j') | KeyCode::Down => Some(1),
        KeyCode::Char('k') | KeyCode::Up => Some(-1),
        _ => None,
    };
    match pane {
        Pane::Chats | Pane::Detail => match code {
            KeyCode::Enter => Some(Action::Act),
            KeyCode::Char('f') if pane == Pane::Detail => Some(Action::ToggleFullscreen),
            other => rows(other).map(Action::Move),
        },
        Pane::Timeline => match code {
            KeyCode::Char('g') => Some(Action::JumpHead),
            KeyCode::Char('G') => Some(Action::JumpTail),
            KeyCode::Char('f') => Some(Action::ToggleFollow),
            KeyCode::Enter => Some(Action::Act),
            other => rows(other).map(Action::Move),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn timeline(code: KeyCode) -> Option<Action> {
        key_event(key(code), Mode::Panes(Pane::Timeline))
    }

    /// The timeline's own keys, plus the ones every pane shares — the paged moves included.
    #[test]
    fn the_timeline_keys_are_the_documented_ones() {
        assert_eq!(timeline(KeyCode::Char('j')), Some(Action::Move(1)));
        assert_eq!(timeline(KeyCode::Up), Some(Action::Move(-1)));
        assert_eq!(timeline(KeyCode::PageDown), Some(Action::Page(1)));
        assert_eq!(timeline(KeyCode::PageUp), Some(Action::Page(-1)));
        assert_eq!(
            key_event(ctrl('d'), Mode::Panes(Pane::Timeline)),
            Some(Action::HalfPage(1))
        );
        assert_eq!(
            key_event(ctrl('u'), Mode::Panes(Pane::Timeline)),
            Some(Action::HalfPage(-1))
        );
        assert_eq!(timeline(KeyCode::Char('g')), Some(Action::JumpHead));
        assert_eq!(timeline(KeyCode::Char('G')), Some(Action::JumpTail));
        assert_eq!(timeline(KeyCode::Char('f')), Some(Action::ToggleFollow));
        assert_eq!(timeline(KeyCode::Enter), Some(Action::Act));
        assert_eq!(timeline(KeyCode::Tab), Some(Action::CyclePane(1)));
        assert_eq!(timeline(KeyCode::BackTab), Some(Action::CyclePane(-1)));
        assert_eq!(timeline(KeyCode::Char('h')), Some(Action::CyclePane(-1)));
        assert_eq!(timeline(KeyCode::Char('l')), Some(Action::CyclePane(1)));
        assert_eq!(timeline(KeyCode::Char('/')), Some(Action::OpenQuery));
        assert_eq!(timeline(KeyCode::Char('?')), Some(Action::ToggleHelp));
        assert_eq!(timeline(KeyCode::Char('q')), Some(Action::Quit));
        assert_eq!(timeline(KeyCode::Esc), Some(Action::Cancel));
        assert_eq!(
            timeline(KeyCode::Char('z')),
            None,
            "an unbound key does nothing"
        );
    }

    /// The detail pane's own keys: Enter folds its long fields, `f` gives it the whole body.
    #[test]
    fn the_detail_pane_keeps_its_own_keys() {
        let detail = |code: KeyCode| key_event(key(code), Mode::Panes(Pane::Detail));
        assert_eq!(detail(KeyCode::Enter), Some(Action::Act));
        assert_eq!(detail(KeyCode::Char('f')), Some(Action::ToggleFullscreen));
        assert_eq!(detail(KeyCode::Char('j')), Some(Action::Move(1)));
        assert_eq!(detail(KeyCode::PageUp), Some(Action::Page(-1)));
        assert_eq!(detail(KeyCode::Char('g')), None, "nothing to jump to here");
    }

    /// The query line owns the keys while it is open: `q` and `?` are text there, `Enter`/`Tab` keep
    /// the terms and `Esc` puts back the ones that were in effect.
    #[test]
    fn the_query_line_owns_the_keys_while_it_is_open() {
        let query = |code: KeyCode| key_event(key(code), Mode::Query);
        for code in [
            KeyCode::Char('q'),
            KeyCode::Char('?'),
            KeyCode::Char('j'),
            KeyCode::Char(' '),
            KeyCode::Char('/'),
        ] {
            assert_eq!(
                query(code),
                Some(Action::QueryKey(key(code))),
                "{code:?} is text in the query"
            );
        }
        assert_eq!(query(KeyCode::Esc), Some(Action::CancelQuery));
        assert_eq!(query(KeyCode::Enter), Some(Action::CloseQuery));
        assert_eq!(query(KeyCode::Tab), Some(Action::CloseQuery));
        assert_eq!(key_event(ctrl('c'), Mode::Query), Some(Action::Quit));
    }

    /// While the help is up it owns the keys; a release is not a press.
    #[test]
    fn the_help_and_a_release_are_handled_apart() {
        assert_eq!(
            key_event(key(KeyCode::Char('j')), Mode::Help),
            None,
            "no key below the help acts on a pane"
        );
        assert_eq!(
            key_event(key(KeyCode::Esc), Mode::Help),
            Some(Action::ToggleHelp)
        );
        assert_eq!(
            key_event(
                KeyEvent::new_with_kind(
                    KeyCode::Char('j'),
                    KeyModifiers::NONE,
                    KeyEventKind::Release
                ),
                Mode::Panes(Pane::Timeline)
            ),
            None
        );
    }

    /// The cycle steps over the detail pane while it shows nothing, and the query is not part of it.
    #[test]
    fn the_cycle_skips_a_closed_detail() {
        assert_eq!(Pane::Timeline.cycle(1, false), Pane::Chats);
        assert_eq!(Pane::Timeline.cycle(1, true), Pane::Detail);
        assert_eq!(Pane::Chats.cycle(-1, false), Pane::Timeline);
    }
}
