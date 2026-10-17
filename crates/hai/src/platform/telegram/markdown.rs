/// MarkdownV2 reserved characters: outside an entity delimiter every one of
/// them must be escaped, `\` included.
const MD_V2_RESERVED: &[char] = &[
    '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
];

/// Text → MarkdownV2 that Telegram can always parse: paired entities (bold,
/// italic, underline, strikethrough, spoiler, code, link) are kept and every
/// other reserved character is escaped, so an unpaired marker becomes literal
/// text instead of failing the send.
pub(super) fn markdown_v2(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        i = match chars[i] {
            '`' => push_code_span(&chars, i, &mut out),
            '[' => push_link(&chars, i, &mut out),
            '*' | '_' | '~' | '|' => push_entity(&chars, i, &mut out),
            '\\' => {
                out.push_str("\\\\");
                i + 1
            }
            c => {
                push_escaped(&mut out, c);
                i + 1
            }
        };
    }
    out
}

fn push_escaped(out: &mut String, c: char) {
    if MD_V2_RESERVED.contains(&c) {
        out.push('\\');
    }
    out.push(c);
}

/// An entity delimiter may open or close only when it is not glued to a word:
/// this is what keeps `snake_case` and `2*3*4` literal.
fn delimited(chars: &[char], start: usize, end: usize) -> bool {
    let before = start.checked_sub(1).and_then(|i| chars.get(i));
    let after = chars.get(end);
    !before.is_some_and(|c| c.is_alphanumeric()) && !after.is_some_and(|c| c.is_alphanumeric())
}

fn push_entity(chars: &[char], start: usize, out: &mut String) -> usize {
    let c = chars[start];
    let doubled = chars.get(start + 1) == Some(&c);
    if c == '|' && !doubled {
        push_escaped(out, c);
        return start + 1;
    }
    let width = if doubled { 2 } else { 1 };
    // Standard Markdown doubles the bold and strikethrough markers (`**`, `~~`); Telegram
    // wants the single-character form, while `__` and `||` are already its own markers.
    let marker = if c == '_' || c == '|' || !doubled {
        c.to_string().repeat(width)
    } else {
        c.to_string()
    };

    if let Some((end, run)) = find_close(chars, start, width, &marker)
        && delimited(chars, start, end + run)
    {
        out.push_str(&marker);
        for c in &chars[start + width..end] {
            push_escaped(out, *c);
        }
        out.push_str(&marker);
        return end + run;
    }

    for _ in 0..width {
        push_escaped(out, c);
    }
    start + width
}

/// Position of the closing marker plus the length of the run it consumed: the
/// close of `*x**` has to swallow both characters.
fn find_close(chars: &[char], start: usize, width: usize, marker: &str) -> Option<(usize, usize)> {
    let marker: Vec<char> = marker.chars().collect();
    let mut i = start + width;
    while i < chars.len() && chars[i] != '\n' {
        if chars[i..].starts_with(marker.as_slice()) {
            let inner = &chars[start + width..i];
            let bounded = inner.first().is_some_and(|c| !c.is_whitespace())
                && inner.last().is_some_and(|c| !c.is_whitespace());
            let run = chars[i..]
                .iter()
                .take_while(|c| **c == chars[start])
                .count();
            if bounded && !inner.windows(marker.len()).any(|w| w == marker.as_slice()) {
                return Some((i, run));
            }
        }
        i += 1;
    }
    None
}

/// Code span: the inline `` ` `` and the fence ``` ``` ```. Inside it only `\` is
/// escaped, plus the backtick itself for an inline span.
fn push_code_span(chars: &[char], start: usize, out: &mut String) -> usize {
    let width = chars[start..].iter().take_while(|c| **c == '`').count();
    let mut i = start + width;
    while i < chars.len() {
        if chars[i] == '`' {
            let run = chars[i..].iter().take_while(|c| **c == '`').count();
            if run >= width {
                out.push_str(&"`".repeat(width));
                for c in &chars[start + width..i] {
                    if *c == '\\' || (width == 1 && *c == '`') {
                        out.push('\\');
                    }
                    out.push(*c);
                }
                out.push_str(&"`".repeat(width));
                return i + width;
            }
            i += run;
        } else {
            i += 1;
        }
    }

    for _ in 0..width {
        out.push_str("\\`");
    }
    start + width
}

/// Link `[text](url)`: kept only for an http/https/tg target, with the text
/// escaped under the entity rules.
fn push_link(chars: &[char], start: usize, out: &mut String) -> usize {
    let text_end = chars[start + 1..]
        .iter()
        .position(|c| *c == ']')
        .map(|i| i + start + 1);
    if let Some(text_end) = text_end
        && chars.get(text_end + 1) == Some(&'(')
        && let Some(close) = chars[text_end + 2..].iter().position(|c| *c == ')')
    {
        let url: String = chars[text_end + 2..text_end + 2 + close].iter().collect();
        let text = &chars[start + 1..text_end];
        if safe_url(&url) && !text.is_empty() {
            out.push('[');
            for c in text {
                push_escaped(out, *c);
            }
            out.push_str("](");
            for c in url.chars() {
                if c == '\\' || c == ')' {
                    out.push('\\');
                }
                out.push(c);
            }
            out.push(')');
            return text_end + 2 + close + 1;
        }
    }

    push_escaped(out, '[');
    start + 1
}

fn safe_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://") || url.starts_with("tg://")
}

#[cfg(test)]
mod tests {
    use super::markdown_v2;

    #[test]
    fn plain_prose_escapes_every_reserved_char() {
        assert_eq!(
            markdown_v2("# Title\n1. item (note)! a-b=c+d>e|f{g}"),
            "\\# Title\n1\\. item \\(note\\)\\! a\\-b\\=c\\+d\\>e\\|f\\{g\\}"
        );
        assert_eq!(markdown_v2("C:\\tmp"), "C:\\\\tmp");
    }

    #[test]
    fn intraword_markers_stay_literal() {
        assert_eq!(markdown_v2("snake_case_name"), "snake\\_case\\_name");
        assert_eq!(markdown_v2("2*3*4"), "2\\*3\\*4");
        assert_eq!(markdown_v2("a_b_"), "a\\_b\\_");
        assert_eq!(markdown_v2("h~x~"), "h\\~x\\~");
    }

    #[test]
    fn unbalanced_markers_stay_literal() {
        assert_eq!(markdown_v2("_oops and *star"), "\\_oops and \\*star");
        assert_eq!(markdown_v2("*open\nclosed*"), "\\*open\nclosed\\*");
        assert_eq!(markdown_v2("|x|"), "\\|x\\|");
    }

    #[test]
    fn balanced_entities_survive_and_normalize() {
        assert_eq!(
            markdown_v2("**bold** and _italic_ and __under__ and ~~gone~~"),
            "*bold* and _italic_ and __under__ and ~gone~"
        );
        assert_eq!(markdown_v2("||spoiler||"), "||spoiler||");
        assert_eq!(markdown_v2("a *b* c"), "a *b* c");
    }

    #[test]
    fn code_spans_keep_reserved_chars_raw() {
        assert_eq!(markdown_v2("run `a_b.c()` now"), "run `a_b.c()` now");
        assert_eq!(
            markdown_v2("```\nlet x = a_b.c();\n```"),
            "```\nlet x = a_b.c();\n```"
        );
        assert_eq!(markdown_v2("open `a.b"), "open \\`a\\.b");
    }

    #[test]
    fn links_keep_known_schemes_only() {
        assert_eq!(
            markdown_v2("[docs](https://example.com/a_b)"),
            "[docs](https://example.com/a_b)"
        );
        assert_eq!(
            markdown_v2("[a*b](https://e.com)"),
            "[a\\*b](https://e.com)"
        );
        assert_eq!(
            markdown_v2("[x](javascript:alert(1))"),
            "\\[x\\]\\(javascript:alert\\(1\\)\\)"
        );
    }
}
