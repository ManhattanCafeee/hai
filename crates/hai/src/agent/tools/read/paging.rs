use std::fmt::Write as _;

use serde_json::{Value, json};

use crate::config::schema::FetchEngine;

pub(super) const DEFAULT_LINES: u64 = 300;
/// One page's line cap for a windowed read, matching the character cap of in-memory text.
pub(super) const MAX_WINDOW_LINES: u64 = 1000;
const MAX_WINDOW_CHARS: usize = 32 * 1024;
/// A line longer than this switches paging to characters: some bodies arrive as one line
/// (minified JSON/JS, single-line feeds).
pub(super) const MAX_LINE_CHARS: usize = 4 * 1024;
pub(super) const MAX_LIST_ENTRIES: u64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Paging {
    Lines,
    Chars,
    Notes,
}

impl Paging {
    fn name(self) -> &'static str {
        match self {
            Paging::Lines => "lines",
            Paging::Chars => "chars",
            Paging::Notes => "notes",
        }
    }

    fn keys(self) -> (&'static str, &'static str) {
        match self {
            Paging::Lines => ("total_lines", "shown_lines"),
            Paging::Chars => ("total_chars", "shown_chars"),
            Paging::Notes => ("total_notes", "shown_notes"),
        }
    }
}

/// `total` and `shown` are counted in `unit`, and `next_offset` is the 1-based position to continue
/// from; it is absent once the page reaches the end.
pub(super) struct Page {
    content: String,
    unit: Paging,
    total: usize,
    shown: usize,
    next_offset: Option<u64>,
    source: Option<FetchEngine>,
}

impl Page {
    fn beyond(unit: Paging, start: usize, total: usize, source: Option<FetchEngine>) -> Self {
        Self {
            content: format!("[offset {} is beyond {total} {}]", start + 1, unit.name()),
            unit,
            total,
            shown: 0,
            next_offset: None,
            source,
        }
    }

    pub(super) fn items(content: String, start: usize, total: usize, shown: usize) -> Self {
        Self::framed(content, Paging::Notes, start, total, shown, None)
    }

    pub(super) fn items_beyond(start: usize, total: usize) -> Self {
        Self::beyond(Paging::Notes, start, total, None)
    }

    /// A page built from a line window a streamed read produced: the counts are the file's, not the
    /// window's.
    pub(super) fn lines(content: String, start: usize, total: usize, shown: usize) -> Self {
        Self::framed(content, Paging::Lines, start, total, shown, None)
    }

    pub(super) fn lines_beyond(start: usize, total: usize) -> Self {
        Self::beyond(Paging::Lines, start, total, None)
    }

    fn framed(
        mut content: String,
        unit: Paging,
        start: usize,
        total: usize,
        shown: usize,
        source: Option<FetchEngine>,
    ) -> Self {
        let end = start + shown;
        let next_offset = (shown > 0 && end < total).then(|| {
            let _ = write!(
                content,
                "\n\n[{shown} of {total} {} shown; continue with offset={}]",
                unit.name(),
                end + 1
            );
            end as u64 + 1
        });
        Self {
            content,
            unit,
            total,
            shown,
            next_offset,
            source,
        }
    }
}

/// `offset` counts from 1 and is interpreted in the `unit` of the returned page: plain text pages
/// by line, text carrying an over-long line pages by character.
pub(super) fn paginate(
    text: &str,
    offset: Option<u64>,
    limit: Option<u64>,
    source: Option<FetchEngine>,
) -> Page {
    let start = offset.unwrap_or(1).max(1) as usize - 1;
    if text
        .lines()
        .any(|line| line.chars().count() > MAX_LINE_CHARS)
    {
        return paginate_chars(text, start, limit, source);
    }
    let total = text.lines().count();
    if start >= total {
        return Page::beyond(Paging::Lines, start, total, source);
    }

    let count = limit.unwrap_or(DEFAULT_LINES).max(1) as usize;
    let mut content = String::new();
    let mut shown = 0usize;
    let mut chars = 0usize;
    for line in text.lines().skip(start).take(count) {
        let added = line.chars().count() + 1;
        if shown > 0 && chars + added > MAX_WINDOW_CHARS {
            break;
        }
        if shown > 0 {
            content.push('\n');
        }
        content.push_str(line);
        chars += added;
        shown += 1;
    }
    Page::framed(content, Paging::Lines, start, total, shown, source)
}

fn paginate_chars(
    text: &str,
    start: usize,
    limit: Option<u64>,
    source: Option<FetchEngine>,
) -> Page {
    let total = text.chars().count();
    if start >= total {
        return Page::beyond(Paging::Chars, start, total, source);
    }
    let window = limit.map_or(MAX_WINDOW_CHARS, |limit| {
        (limit as usize).clamp(1, MAX_WINDOW_CHARS)
    });
    let shown = window.min(total - start);
    let content: String = text.chars().skip(start).take(shown).collect();
    Page::framed(content, Paging::Chars, start, total, shown, source)
}

pub(super) fn page_data(page: Page) -> Value {
    let (total_key, shown_key) = page.unit.keys();
    let mut data = json!({
        "content": page.content,
        "unit": page.unit.name(),
        total_key: page.total,
        shown_key: page.shown,
        "next_offset": page.next_offset,
    });
    if let Some(engine) = page.source {
        data["engine"] = json!(engine);
    }
    data
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::schema::{FetchEngine, WebProvider};

    fn paged(page: Page) -> (String, usize, usize, Option<u64>) {
        (page.content, page.total, page.shown, page.next_offset)
    }

    fn unit_of(page: &Page) -> Paging {
        page.unit
    }

    #[test]
    fn paginate_windows_and_notes_remainder() {
        let text = (1..=5)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            paged(paginate(&text, Some(2), Some(2), None)),
            (
                "2\n3\n\n[2 of 5 lines shown; continue with offset=4]".to_owned(),
                5,
                2,
                Some(4)
            )
        );
        assert_eq!(
            paged(paginate(&text, Some(4), None, None)),
            ("4\n5".to_owned(), 5, 2, None)
        );
    }

    #[test]
    fn paginate_reports_out_of_range_offset() {
        assert_eq!(
            paged(paginate("a\nb", Some(9), None, None)),
            ("[offset 9 is beyond 2 lines]".to_owned(), 2, 0, None)
        );
    }

    #[test]
    fn a_zero_limit_still_reads_a_page() {
        let (content, total, shown, next) = paged(paginate("a\nb", Some(1), Some(0), None));
        assert_eq!((total, shown), (2, 1), "{content}");
        assert_eq!(next, Some(2), "the page continues after what it read");

        let wide = "x".repeat(MAX_LINE_CHARS + 1);
        let (_, total, shown, next) = paged(paginate(&wide, Some(1), Some(0), None));
        assert_eq!((total, shown), (MAX_LINE_CHARS + 1, 1));
        assert_eq!(next, Some(2));
    }

    #[test]
    fn an_empty_page_offers_no_continuation() {
        let (content, _, shown, next) =
            paged(Page::framed(String::new(), Paging::Lines, 0, 5, 0, None));

        assert_eq!(shown, 0);
        assert_eq!(next, None);
        assert!(!content.contains("continue with offset="), "{content}");
    }

    #[test]
    fn wide_single_line_text_pages_by_chars() {
        let tail = 5_000;
        let total_chars = MAX_WINDOW_CHARS + tail;
        let text = "x".repeat(total_chars);

        let first = paginate(&text, None, None, None);
        assert_eq!(unit_of(&first), Paging::Chars);
        let (content, total, shown, next) = paged(first);
        assert_eq!((total, shown), (total_chars, MAX_WINDOW_CHARS));
        assert_eq!(next, Some(MAX_WINDOW_CHARS as u64 + 1));
        assert!(
            content.contains("chars shown; continue with offset="),
            "{content}"
        );

        let (content, _, shown, next) = paged(paginate(&text, next, None, None));
        assert_eq!((shown, next), (tail, None));
        assert!(!content.contains("continue with offset="), "{content}");

        let (content, total, shown, next) =
            paged(paginate(&text, Some(total_chars as u64 + 1), None, None));
        assert_eq!((total, shown, next), (total_chars, 0, None));
        assert!(
            content.contains("beyond") && content.contains("chars]"),
            "{content}"
        );
    }

    #[test]
    fn line_window_shrinks_to_the_char_budget() {
        let text = (0..DEFAULT_LINES)
            .map(|i| format!("{i:0>500}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (content, total, shown, next) = paged(paginate(&text, None, None, None));
        assert_eq!(total, DEFAULT_LINES as usize);
        assert!(
            shown < DEFAULT_LINES as usize,
            "window must shrink to the character budget: {shown}"
        );
        assert_eq!(next, Some(shown as u64 + 1));
        let chars = content.chars().count();
        assert!(chars <= MAX_WINDOW_CHARS + 200, "{chars}");
        assert!(
            chars > MAX_WINDOW_CHARS - 1_000,
            "window should stay near the budget: {chars}"
        );
    }

    #[test]
    fn line_window_keeps_short_lines_within_budget() {
        let text = (0..DEFAULT_LINES)
            .map(|i| format!("{i:0>9}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (content, total, shown, next) = paged(paginate(&text, None, None, None));
        assert_eq!(
            (total, shown, next),
            (DEFAULT_LINES as usize, DEFAULT_LINES as usize, None)
        );
        assert!(content.chars().count() < MAX_WINDOW_CHARS);
    }

    #[test]
    fn page_data_carries_url_source_only_for_fetched_text() {
        let local = page_data(Page::framed(
            "body".to_owned(),
            Paging::Lines,
            0,
            1,
            1,
            None,
        ));
        assert!(local.get("engine").is_none(), "{local}");

        let fetched = page_data(Page::framed(
            "body".to_owned(),
            Paging::Lines,
            0,
            1,
            1,
            Some(FetchEngine::Provider(WebProvider::Parallel)),
        ));
        assert_eq!(fetched["engine"], json!("parallel"), "{fetched}");
        assert!(fetched.get("low_quality").is_none(), "{fetched}");
    }
}
