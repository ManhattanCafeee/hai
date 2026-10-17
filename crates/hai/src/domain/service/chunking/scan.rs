//! Line classification: how a document's lines become blocks before any packing.

use super::{Unit, UnitKind};

#[derive(Debug, Clone, PartialEq)]
enum LineKind {
    /// ATX heading, level 1-6.
    Heading(u8),
    /// Fence opening line with leading whitespace stripped, language tag included.
    FenceOpen(String),
    Table,
    Quote,
    List,
    Text,
    Blank,
}

/// Length of the line's leading marker run (`` ` `` or `~`), or `None` when the line is not a
/// fence: CommonMark requires at least three markers.
fn fence_marker_len(trimmed: &str) -> Option<usize> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let n = trimmed.chars().take_while(|c| *c == first).count();
    (n >= 3).then_some(n)
}

/// ATX heading level (1-6): the `#` run must be followed by a space or the end of the line, so
/// `#foo` is text. `classify_line` and `build_sections` must agree on this rule.
pub(super) fn heading_level(trimmed: &str) -> Option<u8> {
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let after = &trimmed[level..];
    if after.is_empty() || after.starts_with(' ') {
        Some(level as u8)
    } else {
        None
    }
}

fn classify_line(line: &str) -> LineKind {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return LineKind::Blank;
    }
    if fence_marker_len(trimmed).is_some() {
        return LineKind::FenceOpen(trimmed.to_string());
    }
    if let Some(level) = heading_level(trimmed) {
        return LineKind::Heading(level);
    }
    if trimmed.starts_with('|') {
        return LineKind::Table;
    }
    if trimmed.starts_with('>') {
        return LineKind::Quote;
    }
    let list_marker = trimmed
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .count();
    let rest = &trimmed[list_marker..];
    // Markers must be anchored at the start of the line, otherwise prose such as "e.g. text"
    // reads as an ordered list.
    let is_unordered = rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ");
    let is_ordered = {
        let after_digits = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        !after_digits.is_empty()
            && after_digits.len() != rest.len()
            && (after_digits.starts_with(". ") || after_digits.starts_with(") "))
    };
    if is_unordered || is_ordered {
        return LineKind::List;
    }
    LineKind::Text
}

pub(super) fn scan_blocks(content: &str) -> Vec<Unit> {
    let lines: Vec<&str> = content.lines().collect();
    let mut units: Vec<Unit> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        match classify_line(line) {
            LineKind::Blank => i += 1,
            LineKind::Heading(_) => {
                units.push(Unit {
                    kind: UnitKind::Paragraph,
                    text: line.trim_end().to_string(),
                });
                i += 1;
            }
            LineKind::FenceOpen(_) => {
                let open = line;
                let mut body: Vec<&str> = vec![open];
                i += 1;
                while i < lines.len() {
                    let l = lines[i];
                    body.push(l);
                    if is_fence_close(l, open) {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                units.push(Unit {
                    kind: UnitKind::Code,
                    text: body.join("\n"),
                });
            }
            LineKind::Table => {
                let mut body: Vec<&str> = vec![line];
                i += 1;
                while i < lines.len() && classify_line(lines[i]) == LineKind::Table {
                    body.push(lines[i]);
                    i += 1;
                }
                units.push(Unit {
                    kind: UnitKind::Table,
                    text: body.join("\n"),
                });
            }
            LineKind::Quote => {
                let mut body: Vec<&str> = vec![line];
                i += 1;
                while i < lines.len() && classify_line(lines[i]) == LineKind::Quote {
                    body.push(lines[i]);
                    i += 1;
                }
                units.push(Unit {
                    kind: UnitKind::Quote,
                    text: body.join("\n"),
                });
            }
            LineKind::List => {
                let mut body: Vec<&str> = vec![line];
                i += 1;
                while i < lines.len() && classify_line(lines[i]) == LineKind::List {
                    body.push(lines[i]);
                    i += 1;
                }
                units.push(Unit {
                    kind: UnitKind::List,
                    text: body.join("\n"),
                });
            }
            LineKind::Text => {
                let mut body: Vec<&str> = vec![line];
                i += 1;
                while i < lines.len() {
                    match classify_line(lines[i]) {
                        LineKind::Text => body.push(lines[i]),
                        LineKind::Blank => break,
                        _ => break,
                    }
                    i += 1;
                }
                units.push(Unit {
                    kind: UnitKind::Paragraph,
                    text: body.join("\n"),
                });
            }
        }
    }
    units
}

/// Closing fence: indentation is tolerated (CommonMark allows an indented closing fence), the line
/// holds nothing but the marker character, and it is at least as long as the opening run — the
/// opening line's language tag does not count.
fn is_fence_close(line: &str, open: &str) -> bool {
    let trimmed = line.trim_end();
    let content = trimmed.trim_start();
    let open_trim = open.trim_start();
    let Some(marker) = open_trim.chars().next() else {
        return false;
    };
    if marker != '`' && marker != '~' {
        return false;
    }
    let open_len = open_trim.chars().take_while(|c| *c == marker).count();
    let close_len = content.chars().take_while(|c| *c == marker).count();
    close_len >= open_len && content.chars().all(|c| c == marker)
}
