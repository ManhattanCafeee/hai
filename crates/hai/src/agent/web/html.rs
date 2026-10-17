//! HTML article extraction (Readability, dropping nav/sidebars/footers) and tag stripping for
//! search snippets.

use dom_smoothie::{Config, ParsePolicy, Readability, TextMode};

use crate::error::{AppError, Result};

/// Errors when the page yields no article text. Whitespace runs are collapsed and entities decoded
/// by the parser, so callers get plain text, not markup.
pub fn render_text(html: &str) -> Result<String> {
    let config = Config {
        text_mode: TextMode::Formatted,
        ..Config::default()
    };
    Readability::new(html, None, Some(config))
        .and_then(|mut readability| readability.parse_with_policy(ParsePolicy::Strict))
        .map(|article| article.text_content.to_string())
        .map_err(|e| AppError::message(format!("Failed to extract article text: {e}")))
}

pub fn strip_tags(fragment: &str) -> String {
    let text = dom_query::Document::from(fragment)
        .select("body")
        .text()
        .to_string();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARTICLE: &str = "<p>Rust is a systems programming language focused on safety and performance, widely used for command line tools and services.</p>\
<p>The compiler enforces memory safety without a garbage collector, which removes a whole class of bugs at compile time.</p>";

    /// The article body sits inside `<article>`; `chrome` is the navigation/footer around it.
    fn page(inner: &str, chrome: &str) -> String {
        format!(
            "<html><head><title>Doc</title></head><body>{chrome}<article>{inner}</article></body></html>"
        )
    }

    #[test]
    fn render_text_drops_chrome_and_keeps_article() {
        let html = page(
            ARTICLE,
            "<nav><a href=\"/\">Home</a><a href=\"/about\">About us</a></nav>\
             <footer>Copyright 2026 Example Inc</footer>",
        );
        let text = render_text(&html).expect("text");

        assert!(text.contains("memory safety"), "{text}");
        assert!(!text.contains("About us"), "{text}");
        assert!(!text.contains("Copyright 2026"), "{text}");
    }

    #[test]
    fn render_text_decodes_entities_and_keeps_unicode() {
        let html = page(
            &format!(
                "{ARTICLE}<p>Naïve café résumé: A &amp; B &mdash; C &#39;quoted&#39; — long enough prose to be treated as the article body.</p>"
            ),
            "",
        );
        let text = render_text(&html).expect("text");

        assert!(text.contains("Naïve café résumé"), "{text}");
        assert!(text.contains("A & B — C 'quoted'"), "{text}");
        assert!(!text.contains("  "), "{text}");
    }

    #[test]
    fn render_text_returns_text_for_tiny_page() {
        let text = render_text("<html><body><p>hi there</p></body></html>").expect("text");
        assert!(text.contains("hi there"), "{text}");
    }

    #[test]
    fn render_text_errors_on_empty_page() {
        let err = render_text("<html><body></body></html>").expect_err("empty page");
        assert!(err.to_string().contains("extract article text"), "{err}");
    }

    #[test]
    fn strip_tags_cleans_fragment() {
        assert_eq!(
            strip_tags("<b>Rust</b> &amp; Cargo\n  workspace"),
            "Rust & Cargo workspace"
        );
    }
}
