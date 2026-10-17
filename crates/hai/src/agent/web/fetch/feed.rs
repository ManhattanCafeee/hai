//! Feed (RSS 2.0 / Atom) → item text, one of `read`'s body shapes: it keeps a tag stream from
//! flooding the context.

use std::fmt::Write as _;

use roxmltree::{Document, Node};

use super::super::html;
use crate::util::text::truncate_chars;

const MAX_ITEMS: usize = 50;
const MAX_SUMMARY_CHARS: usize = 240;

/// `None` when the document is not a feed or does not parse, so the caller keeps the raw body.
pub(super) fn render(xml: &str) -> Option<String> {
    let doc = Document::parse(xml).ok()?;
    let root = doc.root_element();
    let (items, channel) = match root.tag_name().name() {
        "rss" => (
            descendants(root, "item"),
            root.descendants().find(|n| n.has_tag_name("channel")),
        ),
        "feed" => (descendants(root, "entry"), Some(root)),
        _ => return None,
    };
    if items.is_empty() {
        return None;
    }

    let mut out = String::new();
    if let Some(channel) = channel
        && let Some(title) = field(&channel, "title")
    {
        out.push_str(&title);
        let blurb = field(&channel, "subtitle").or_else(|| field(&channel, "description"));
        if let Some(blurb) = blurb {
            out.push_str(" — ");
            out.push_str(&blurb);
        }
        out.push_str("\n\n");
    }

    for (index, item) in items.iter().take(MAX_ITEMS).enumerate() {
        let title = field(item, "title").unwrap_or_default();
        let _ = write!(out, "{}. {title}", index + 1);
        if let Some(date) = field(item, "pubDate")
            .or_else(|| field(item, "updated"))
            .or_else(|| field(item, "published"))
        {
            let _ = write!(out, " ({date})");
        }
        out.push('\n');
        if let Some(link) = link_of(item) {
            out.push_str("   ");
            out.push_str(&link);
            out.push('\n');
        }
        let summary = field(item, "description")
            .or_else(|| field(item, "summary"))
            .or_else(|| field(item, "content"));
        if let Some(summary) = summary {
            out.push_str("   ");
            out.push_str(&truncate_chars(&summary, MAX_SUMMARY_CHARS));
            out.push('\n');
        }
        out.push('\n');
    }
    if items.len() > MAX_ITEMS {
        let _ = write!(out, "\n\n[{MAX_ITEMS} of {} items shown]", items.len());
    }
    Some(out.trim_end().to_owned())
}

fn descendants<'a, 'input>(root: Node<'a, 'input>, name: &str) -> Vec<Node<'a, 'input>> {
    root.descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == name)
        .collect()
}

fn field(node: &Node<'_, '_>, name: &str) -> Option<String> {
    let raw = node
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == name)?
        .text()?;
    let cleaned = html::strip_tags(raw);
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| cleaned.to_owned())
}

/// Item link: RSS puts it in the child element text, Atom in `<link href>` (`rel="alternate"`
/// preferred, any other relation used only as a fallback).
fn link_of(node: &Node<'_, '_>) -> Option<String> {
    if let Some(link) = field(node, "link") {
        return Some(link);
    }
    let links: Vec<Node<'_, '_>> = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "link")
        .collect();
    let alternate = links
        .iter()
        .find(|link| link.attribute("rel").is_none_or(|rel| rel == "alternate"))
        .or_else(|| links.first())?;
    alternate.attribute("href").map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <title>Hacker News</title>
  <description>Links for the intellectually curious</description>
  <item>
    <title>Can gzip be a language model?</title>
    <link>https://nathan.rs/posts/gzip-lm/</link>
    <pubDate>Mon, 22 Sep 2026 09:00:00 GMT</pubDate>
    <description><![CDATA[<p>Some <b>markup</b> here</p>]]></description>
  </item>
  <item>
    <title>MiMo v2.6</title>
    <link>https://xiaomi.com/mimo</link>
  </item>
</channel></rss>"#;

    const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Rust Blog</title>
  <subtitle>Official Rust blog</subtitle>
  <entry>
    <title>Async closures MVP</title>
    <link rel="alternate" href="https://blog.rust-lang.org/inside-rust/async-closures"/>
    <updated>2026-08-09T00:00:00Z</updated>
    <summary>Call for testing</summary>
  </entry>
</feed>"#;

    #[test]
    fn rss_becomes_an_item_list() {
        let text = render(RSS).expect("feed");
        assert!(
            text.starts_with("Hacker News — Links for the intellectually curious"),
            "{text}"
        );
        assert!(
            text.contains("1. Can gzip be a language model? (Mon, 22 Sep 2026 09:00:00 GMT)"),
            "{text}"
        );
        assert!(text.contains("https://nathan.rs/posts/gzip-lm/"), "{text}");
        assert!(text.contains("Some markup here"), "{text}");
        assert!(!text.contains("<b>"), "{text}");
        assert!(text.contains("2. MiMo v2.6"), "{text}");
        assert!(!text.contains("<link>"), "{text}");
    }

    #[test]
    fn atom_uses_link_href_and_updated() {
        let text = render(ATOM).expect("feed");
        assert!(text.contains("Rust Blog — Official Rust blog"), "{text}");
        assert!(
            text.contains("1. Async closures MVP (2026-08-09T00:00:00Z)"),
            "{text}"
        );
        assert!(
            text.contains("https://blog.rust-lang.org/inside-rust/async-closures"),
            "{text}"
        );
        assert!(text.contains("Call for testing"), "{text}");
    }

    #[test]
    fn item_list_reports_truncation() {
        let items: String = (0..=MAX_ITEMS)
            .map(|index| format!("<item><title>t{index}</title></item>"))
            .collect();
        let text = render(&format!(
            "<rss version=\"2.0\"><channel><title>F</title>{items}</channel></rss>"
        ))
        .expect("feed");
        assert!(
            text.ends_with(&format!("[{MAX_ITEMS} of {} items shown]", MAX_ITEMS + 1)),
            "{text}"
        );

        assert!(!render(RSS).expect("feed").contains("items shown"));
    }

    #[test]
    fn non_feed_and_broken_xml_are_rejected() {
        assert_eq!(render("<html><body>hi</body></html>"), None);
        assert_eq!(render("<rss><channel></channel></rss>"), None);
        assert_eq!(render("<rss><channel><item><title>x"), None);
    }
}
