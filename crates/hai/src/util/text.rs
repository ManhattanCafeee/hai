use linkify::LinkFinder;

/// Truncate to `max` characters without splitting one; a truncated text gains an ellipsis.
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((idx, _)) => format!("{}…", &text[..idx]),
        None => text.to_owned(),
    }
}

/// Extract URL links only; `LinkKind::Url` skips e-mails and the other link kinds.
pub(crate) fn extract_urls(text: &str) -> Vec<String> {
    let mut finder = LinkFinder::new();
    finder
        .kinds(&[linkify::LinkKind::Url])
        .links(text)
        .map(|link| link.as_str().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_keeps_char_boundaries() {
        assert_eq!(truncate_chars("中文段落", 2), "中文…");
        assert_eq!(truncate_chars("abc", 3), "abc");
        assert_eq!(truncate_chars("abcd", 3), "abc…");
    }
}
