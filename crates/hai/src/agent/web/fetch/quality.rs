const MIN_TEXT_CHARS: usize = 100;
const SHELL_MARKER_MAX_BYTES: usize = 1024;
const UNRENDERED_MARKERS: [&str; 5] = [
    "enable javascript",
    "javascript required",
    "turn on javascript",
    "please enable javascript",
    "browser not supported",
];
const NAV_LINE_CHARS: usize = 40;
const NAV_SHORT_RATIO_NUM: usize = 7;
const NAV_MIN_LINES: usize = 10;
pub(super) fn usable_text(text: &str) -> bool {
    text.chars().filter(|c| !c.is_whitespace()).count() > MIN_TEXT_CHARS && !looks_like_shell(text)
}

fn looks_like_shell(text: &str) -> bool {
    let lower = text.to_lowercase();
    if text.len() < SHELL_MARKER_MAX_BYTES
        && UNRENDERED_MARKERS
            .iter()
            .any(|marker| lower.contains(marker))
    {
        return true;
    }
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let short = lines
        .iter()
        .filter(|line| line.trim().chars().count() < NAV_LINE_CHARS)
        .count();
    lines.len() > NAV_MIN_LINES && short * 10 > lines.len() * NAV_SHORT_RATIO_NUM
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usable_text_rejects_shells_and_navigation_dumps() {
        let menu: String = (0..20).map(|i| format!("Nav item {i:02}\n")).collect();
        assert!(menu.chars().filter(|c| !c.is_whitespace()).count() > MIN_TEXT_CHARS);
        assert!(!usable_text(&menu), "a navigation dump is not article text");

        assert!(!usable_text("Reddit"));
        assert!(!usable_text(
            &"Please enable JavaScript to continue. ".repeat(3)
        ));

        let article = "Rust's ownership rules are checked at compile time, so the runtime needs no garbage collector.";
        assert!(usable_text(&article.repeat(4)));
    }
}
