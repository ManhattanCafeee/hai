use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use crate::{
    cli::display::EventDisplay,
    domain::vo::{EventKind, TurnNumber},
};

/// The one-line query: what was typed, what it means, and which rows it keeps. `parsed` always holds
/// the last query that parsed, so a half-typed term leaves the window alone and only `error` moves.
/// The line is not part of the TUI until it is opened, and it holds the keyboard only while it is.
pub(super) struct Query {
    text: String,
    cursor: usize,
    pub parsed: Terms,
    pub error: Option<String>,
    /// What was in effect when the line was opened: what `Esc` puts back.
    saved: String,
    editing: bool,
}

impl Query {
    pub fn new(seed: String) -> Self {
        let mut query = Self {
            text: seed,
            cursor: 0,
            parsed: Terms::default(),
            error: None,
            saved: String::new(),
            editing: false,
        };
        query.cursor = query.text.len();
        query.reparse();
        query
    }

    /// Take the keyboard for the query line.
    pub fn begin(&mut self) {
        self.saved = self.text.clone();
        self.editing = true;
    }

    pub fn is_editing(&self) -> bool {
        self.editing
    }

    /// Keep what was typed and give the keyboard back. A term that does not parse leaves the one that
    /// does in effect, and the message stays on the error line.
    pub fn commit(&mut self) {
        self.editing = false;
    }

    /// Give the keyboard back and put back the terms that were in effect when it was opened.
    pub fn cancel(&mut self) {
        if self.text != self.saved {
            self.text = std::mem::take(&mut self.saved);
            self.cursor = self.text.len();
            self.reparse();
        }
        self.editing = false;
    }

    /// Text editing only: what a key means for the window is the reducer's business, and an
    /// unparsable query leaves the terms in force.
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        match key.code {
            KeyCode::Left => Self::move_cursor(&self.text, &mut self.cursor, -1),
            KeyCode::Right => Self::move_cursor(&self.text, &mut self.cursor, 1),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.len(),
            KeyCode::Backspace => Self::backspace(&mut self.text, &mut self.cursor),
            KeyCode::Delete => Self::delete(&mut self.text, &mut self.cursor),
            // A control chord is never text: `Ctrl+U`/`Ctrl+D` would otherwise land as a `u` or a `d`.
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            _ => return,
        }
        self.reparse();
    }

    /// Put `chat:<id>` in the query, or drop the term when it is already in effect.
    pub fn toggle_chat(&mut self, chat: i64) {
        let keep = self.parsed.chat != Some(chat);
        let mut tokens: Vec<String> = self
            .text
            .split_whitespace()
            .filter(|token| !token.starts_with("chat:"))
            .map(str::to_string)
            .collect();
        if keep {
            tokens.push(format!("chat:{chat}"));
        }
        self.text = tokens.join(" ");
        self.cursor = self.text.len();
        self.reparse();
    }

    /// The terms the window was queried with and the terms the query line holds have to agree for the
    /// rows to be the reader's; a mismatch is what queues a reload.
    pub fn stale_sql(&self, chat_filter: Option<i64>, kind_filter: Option<EventKind>) -> bool {
        chat_filter != self.parsed.chat || kind_filter != self.parsed.kind
    }

    fn reparse(&mut self) {
        match Terms::parse(&self.text) {
            Ok(terms) => {
                self.parsed = terms;
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }

    /// One line, with the caret: it is drawn only while it holds the keyboard.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let red = self.error.is_some();
        let prompt_style = if red {
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        };
        let text_style = if red {
            Style::new().fg(Color::Red)
        } else {
            Style::new().fg(Color::White)
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" query ", prompt_style),
                Span::styled(self.text.as_str(), text_style),
            ])),
            area,
        );
        let caret = self.cursor.min(self.text.len());
        let x =
            area.x + " query ".len() as u16 + UnicodeWidthStr::width(&self.text[..caret]) as u16;
        f.set_cursor_position((x, area.y));
    }

    /// Move the cursor — a byte index — to the next char boundary.
    fn move_cursor(s: &str, cur: &mut usize, delta: isize) {
        let len = s.len();
        let c = (*cur).min(len);
        if delta < 0 {
            let mut i = c;
            while i > 0 {
                i -= 1;
                if s.is_char_boundary(i) {
                    break;
                }
            }
            *cur = i;
        } else if delta > 0 {
            let mut i = c;
            while i < len {
                i += 1;
                if s.is_char_boundary(i) {
                    break;
                }
            }
            *cur = i.min(len);
        }
    }

    fn backspace(s: &mut String, cur: &mut usize) {
        let c = (*cur).min(s.len());
        let mut i = c;
        while i > 0 {
            i -= 1;
            if s.is_char_boundary(i) {
                break;
            }
        }
        if i < c {
            s.drain(i..c);
            *cur = i;
        }
    }

    fn delete(s: &mut String, cur: &mut usize) {
        let c = (*cur).min(s.len());
        if c >= s.len() {
            return;
        }
        let mut i = c + 1;
        while i < s.len() && !s.is_char_boundary(i) {
            i += 1;
        }
        s.drain(c..i);
    }
}

/// What a query line asks for. `chat` and `kind` are the two terms the database can narrow by; the
/// rest are matched against an `EventDisplay` in memory.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Terms {
    /// Bare words, joined by a space: searched for, case-insensitively, in each row's rendering.
    pub text: String,
    pub chat: Option<i64>,
    pub kind: Option<EventKind>,
    /// A glob: `*` matches any run, `?` one character.
    pub tool: Option<String>,
    pub turn: Option<TurnNumber>,
    pub only_errors: bool,
    /// Milliseconds a turn has to have taken to survive.
    pub slow: Option<u64>,
}

impl Terms {
    /// Read a query line. A token carrying a colon is a keyed term, `err` is the one flag, and
    /// anything else — an unknown key included — is text to search for: a reader looking for the
    /// literal string `mode:hunt` should get the rows that say it.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut terms = Self::default();
        let mut words: Vec<&str> = Vec::new();
        for token in text.split_whitespace() {
            let Some((key, value)) = token.split_once(':') else {
                if token == "err" {
                    terms.only_errors = true;
                } else {
                    words.push(token);
                }
                continue;
            };
            match key {
                "chat" => terms.chat = Some(parse_chat(value)?),
                "kind" => terms.kind = Some(parse_kind(value)?),
                "tool" => terms.tool = Some(value.to_string()),
                "turn" => terms.turn = Some(parse_turn(value)?),
                "slow" => terms.slow = Some(parse_duration(value)?),
                _ => words.push(token),
            }
        }
        terms.text = words.join(" ");
        Ok(terms)
    }

    /// Whether one row survives: every term a row can fail, it fails here.
    pub fn matches(&self, seq: i64, display: &EventDisplay) -> bool {
        if let Some(chat) = self.chat
            && display.chat_id != chat
        {
            return false;
        }
        if let Some(kind) = self.kind
            && display.kind != Some(kind)
        {
            return false;
        }
        if let Some(tool) = &self.tool
            && !display
                .facts
                .tool
                .as_deref()
                .is_some_and(|called| glob_matches(tool, called))
        {
            return false;
        }
        if let Some(turn) = self.turn
            && display.facts.turn != Some(turn)
        {
            return false;
        }
        if self.only_errors && !failed(display) {
            return false;
        }
        if let Some(ms) = self.slow
            && !display
                .facts
                .outcome
                .is_some_and(|outcome| outcome.elapsed_ms.is_some_and(|elapsed| elapsed >= ms))
        {
            return false;
        }
        if !self.text.is_empty() {
            let needle = self.text.to_lowercase();
            let haystack = format!(
                "{} {seq} {}",
                display.one_liner.to_lowercase(),
                display.chat_id
            );
            if !haystack.contains(&needle) {
                return false;
            }
        }
        true
    }

    /// The terms as this pane spells them: what the reader can see is in effect, which is not the
    /// same as what they typed (a token the grammar does not know is a search, not a filter).
    pub fn spelling(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(chat) = self.chat {
            parts.push(format!("chat:{chat}"));
        }
        if let Some(kind) = self.kind {
            parts.push(format!("kind:{}", kind.as_str()));
        }
        if let Some(tool) = &self.tool {
            parts.push(format!("tool:{tool}"));
        }
        if let Some(turn) = self.turn {
            parts.push(format!("turn:{}", turn.0));
        }
        if self.only_errors {
            parts.push(String::from("err"));
        }
        if let Some(ms) = self.slow {
            parts.push(format!("slow:{ms}ms"));
        }
        if !self.text.is_empty() {
            parts.push(format!("{:?}", self.text));
        }
        parts.join("  ")
    }
}

/// A turn that ended badly, by its own end record or by a tool that reported failure.
fn failed(display: &EventDisplay) -> bool {
    display.facts.ok == Some(false) || display.facts.outcome.is_some_and(|outcome| outcome.failed)
}

fn parse_chat(value: &str) -> Result<i64, String> {
    match value.parse::<i64>() {
        Ok(id) if id != 0 => Ok(id),
        _ => Err(format!("chat: {value} is not a chat id")),
    }
}

fn parse_kind(value: &str) -> Result<EventKind, String> {
    value
        .parse::<EventKind>()
        .map_err(|_| format!("kind: {value} is not an event kind"))
}

fn parse_turn(value: &str) -> Result<TurnNumber, String> {
    value
        .parse::<usize>()
        .map(TurnNumber)
        .map_err(|_| format!("turn: {value} is not a turn number"))
}

fn parse_duration(value: &str) -> Result<u64, String> {
    let (digits, millis) = if let Some(digits) = value.strip_suffix("ms") {
        (digits, true)
    } else if let Some(digits) = value.strip_suffix('s') {
        (digits, false)
    } else {
        return Err(format!(
            "slow: {value} is not a duration such as 2s or 500ms"
        ));
    };
    match digits.parse::<u64>() {
        Ok(n) if millis => Ok(n),
        Ok(n) => Ok(n * 1000),
        Err(_) => Err(format!(
            "slow: {value} is not a duration such as 2s or 500ms"
        )),
    }
}

/// A glob match over `*` (any run) and `?` (one character), with the last `*` remembered so a
/// mismatch can be retried one character further on.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;

    use super::*;
    use crate::cli::display::{EventFacts, TurnOutcome};

    fn display(
        chat_id: i64,
        turn: Option<usize>,
        tool: &str,
        outcome: Option<TurnOutcome>,
    ) -> EventDisplay {
        EventDisplay {
            tag: "T",
            one_liner: String::from("TOOL  5.1  send_message(sent)  ✓"),
            sections: Vec::new(),
            chat_id,
            kind: Some(EventKind::ToolCallResult),
            color: (0, 0, 0),
            facts: EventFacts {
                turn: turn.map(TurnNumber),
                tool: Some(tool.to_string()),
                ok: Some(true),
                outcome,
            },
        }
    }

    fn turn_of(ms: u64, failed: bool) -> TurnOutcome {
        TurnOutcome {
            tools: Some(1),
            elapsed_ms: Some(ms),
            failed,
            replied: true,
        }
    }

    fn matches(query: &str, row: &EventDisplay) -> bool {
        Terms::parse(query)
            .expect("a query that parses")
            .matches(1, row)
    }

    /// Every term the grammar knows reads the field it names, and the text half searches the row's
    /// rendering along with its seq and chat.
    #[test]
    fn a_query_parses_every_term() {
        let terms =
            Terms::parse("chat:4242 kind:tool-call-result tool:send_* turn:5 slow:1s hello")
                .expect("a query that parses");
        assert_eq!(terms.chat, Some(4242));
        assert_eq!(terms.kind, Some(EventKind::ToolCallResult));
        assert_eq!(terms.tool.as_deref(), Some("send_*"));
        assert_eq!(terms.turn, Some(TurnNumber(5)));
        assert_eq!(terms.slow, Some(1000));
        assert_eq!(terms.text, "hello");
        assert!(!terms.only_errors);

        assert!(
            Terms::parse("err")
                .expect("a query that parses")
                .only_errors
        );
        assert_eq!(
            Terms::parse("slow:500ms")
                .expect("a query that parses")
                .slow,
            Some(500)
        );
    }

    /// The grammar is forgiving: a value that cannot be read is refused, a token it does not know is
    /// a literal to search for.
    #[test]
    fn an_unknown_key_is_text_and_a_bad_value_is_an_error() {
        for bad in [
            "chat:x",
            "chat:0",
            "slow:abc",
            "slow:1 minute",
            "kind:nope",
            "turn:x",
        ] {
            assert!(Terms::parse(bad).is_err(), "{bad} should not parse");
        }

        let terms = Terms::parse("weird:1 slow").expect("a query that parses");
        assert_eq!(terms.text, "weird:1 slow");
        assert_eq!(terms.slow, None);
        assert_eq!(terms.chat, None);
    }

    /// A tool term is a glob: the family matches, an unrelated tool does not.
    #[test]
    fn a_tool_glob_matches_its_family() {
        let row = display(7, Some(5), "send_message", None);
        assert!(matches("tool:send_*", &row));
        assert!(matches("tool:send_message", &row));
        assert!(matches("tool:send_m?ssage", &row));
        assert!(!matches("tool:read_*", &row));
        assert!(!matches("tool:send", &row));
    }

    /// `err` and `slow:` read the outcome, which only a turn's end record carries.
    #[test]
    fn error_and_slow_terms_read_the_outcome() {
        let failed = display(7, Some(5), "send_message", Some(turn_of(1200, true)));
        let fine = display(7, Some(5), "send_message", Some(turn_of(1200, false)));
        let running = display(7, Some(5), "send_message", None);

        assert!(matches("err", &failed));
        assert!(!matches("err", &fine));
        assert!(matches("slow:1s", &failed));
        assert!(!matches("slow:2s", &fine));
        assert!(
            !matches("slow:1s", &running),
            "a turn still running has no duration"
        );
    }

    /// A text term never matches a row whose rendering does not carry it, and the rest of the terms
    /// still have to hold.
    #[test]
    fn a_text_term_searches_the_rendering() {
        let row = display(4242, Some(5), "send_message", Some(turn_of(1200, false)));
        assert!(matches("send_message", &row));
        assert!(matches("4242", &row), "the chat id is part of the haystack");
        assert!(!matches("read_file", &row));
        assert!(matches("chat:4242 send_message", &row));
        assert!(!matches("chat:7 send_message", &row));
    }

    /// A query that stops parsing keeps the terms it had, so a half-typed value never disturbs the
    /// window the reader is looking at.
    #[test]
    fn a_broken_query_keeps_the_last_valid_terms() {
        let mut query = Query::new(String::from("chat:4242 "));
        assert_eq!(query.parsed.chat, Some(4242));
        assert!(query.error.is_none());

        for c in "chat:x".chars() {
            query.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }

        assert!(query.error.is_some(), "the half-typed value is refused");
        assert_eq!(query.parsed.chat, Some(4242), "the old terms stay");
    }

    /// A control chord is never text: `Ctrl+U` would otherwise land in the line as a `u`.
    #[test]
    fn a_control_chord_is_not_typed_into_the_query() {
        let mut query = Query::new(String::new());
        query.begin();
        query.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        query.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        query.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(query.text, "x");
    }

    /// Picking a chat writes the term into the line, and picking it again takes it out; one chat
    /// term lives in the line at a time.
    #[test]
    fn picking_a_chat_writes_the_term() {
        let mut query = Query::new(String::new());
        query.toggle_chat(4242);
        assert_eq!(query.text, "chat:4242");
        assert_eq!(query.parsed.chat, Some(4242));

        query.toggle_chat(777);
        assert_eq!(query.text, "chat:777");

        query.toggle_chat(777);
        assert_eq!(query.text, "");
        assert_eq!(query.parsed.chat, None);
    }
}
