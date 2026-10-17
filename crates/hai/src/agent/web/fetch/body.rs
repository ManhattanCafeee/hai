//! Response-body classification and shaping, per class, into model-readable text.

use mime::Mime;
use reqwest::header::CONTENT_TYPE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Body {
    Image,
    Audio,
    Video,
    Html,
    Json,
    Xml,
    Text,
    Unsupported,
}

fn has_type_or_suffix(mime: &Mime, name: &str) -> bool {
    mime.subtype().as_str() == name || mime.suffix().is_some_and(|suffix| suffix.as_str() == name)
}

impl Body {
    /// The class a response's own media type declares.
    pub(super) fn of_mime(mime: &Mime) -> Self {
        match mime.type_() {
            mime::IMAGE if has_type_or_suffix(mime, "xml") => Self::Xml,
            mime::IMAGE => Self::Image,
            mime::AUDIO => Self::Audio,
            mime::VIDEO => Self::Video,
            mime::TEXT if mime.subtype() == mime::HTML => Self::Html,
            mime::TEXT if has_type_or_suffix(mime, "xml") => Self::Xml,
            mime::TEXT => Self::Text,
            mime::APPLICATION if mime.subtype().as_str() == "xhtml" => Self::Html,
            mime::APPLICATION if has_type_or_suffix(mime, "json") => Self::Json,
            mime::APPLICATION if has_type_or_suffix(mime, "xml") => Self::Xml,
            mime::APPLICATION if mime.subtype().as_str() == "javascript" => Self::Text,
            _ => Self::Unsupported,
        }
    }
}

/// What a response without a media type is classified as: nothing, so its URL's extension decides.
const UNKNOWN_TYPE: Mime = mime::APPLICATION_OCTET_STREAM;

pub(super) fn content_type(resp: &reqwest::Response) -> Mime {
    resp.headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(UNKNOWN_TYPE)
}

/// JSON → indented text. `None` on a parse failure so the caller keeps the raw body rather than
/// guessing at a format.
pub(super) fn pretty_json(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

/// Decodes with the charset declared by the response; without one it falls back to UTF-8, where
/// `encoding_rs` still honors a UTF-8 BOM.
pub(super) fn decode(bytes: &[u8], mime: &Mime) -> String {
    let encoding = mime
        .get_param(mime::CHARSET)
        .and_then(|c| encoding_rs::Encoding::for_label(c.as_str().as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    encoding.decode(bytes).0.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn of_mime_dispatches_by_content_type() {
        for (mime, expected) in [
            ("image/png", Body::Image),
            ("image/svg+xml", Body::Xml),
            ("audio/mpeg", Body::Audio),
            ("audio/ogg", Body::Audio),
            ("video/mp4", Body::Video),
            ("video/webm", Body::Video),
            ("text/html; charset=utf-8", Body::Html),
            ("application/xhtml+xml", Body::Html),
            ("application/json", Body::Json),
            ("application/ld+json", Body::Json),
            ("application/xml", Body::Xml),
            ("application/rss+xml", Body::Xml),
            ("text/xml", Body::Xml),
            ("text/markdown; charset=utf-8", Body::Text),
            ("text/plain", Body::Text),
            ("application/javascript", Body::Text),
            ("application/pdf", Body::Unsupported),
        ] {
            assert_eq!(Body::of_mime(&mime.parse().unwrap()), expected, "{mime}");
        }
    }

    #[test]
    fn an_absent_content_type_is_not_text() {
        assert_eq!(Body::of_mime(&UNKNOWN_TYPE), Body::Unsupported);
    }
    #[test]
    fn json_becomes_indented_text_or_stays_raw() {
        let pretty = pretty_json(r#"{"a":[1,2],"b":"x"}"#).expect("valid json");
        assert!(pretty.contains("\n"), "{pretty}");
        assert!(pretty.contains("\"a\": ["), "{pretty}");
        assert_eq!(pretty_json("{"), None);
    }
    #[test]
    fn decode_respects_declared_charset() {
        let gbk = encoding_rs::GBK.encode("中文").0.into_owned();
        let mime: Mime = "text/html; charset=gbk".parse().unwrap();
        assert_eq!(decode(&gbk, &mime), "中文");
        let utf8 = "中文".as_bytes();
        assert_eq!(decode(utf8, &"text/plain".parse().unwrap()), "中文");
    }
}
