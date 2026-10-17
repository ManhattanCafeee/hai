//! HTTP fetch: URL → body text, or an image URL handed to vision analysis.

mod body;
mod feed;
mod quality;

use std::{sync::Arc, time::Duration};

use mime::Mime;

use self::{
    body::{Body, content_type, decode, pretty_json},
    quality::usable_text,
};
use super::{
    WebUrl, html,
    provider::{FetchStep, Providers},
};
use crate::{
    agent::multimodal::{ImageSource, MediaSource, Payload},
    config::schema::{FetchEngine, WebFetchConfig},
    domain::vo::{AttachmentParser, ImageFormat, MediaCodec},
    error::{AppError, Result, bail},
    util::text::truncate_chars,
};

const ACCEPT: &str = "text/markdown, text/html;q=0.9, application/json;q=0.9, application/xml;q=0.9, \
                      text/plain;q=0.8, image/*;q=0.8, */*;q=0.5";
pub enum Fetched {
    Text {
        text: String,
        engine: FetchEngine,
    },
    Image {
        url: String,
    },
    /// Audio the caller hands to the model inline: the provider's audio shape carries data, not a URL.
    Audio {
        data: Vec<u8>,
        codec: Option<MediaCodec>,
    },
    /// Video the model fetches itself, so its bytes never pass through here.
    Video {
        url: String,
    },
}

impl Fetched {
    /// What the multimodal analyzer reads, when this fetch is not text: the payload and the parser
    /// that reads it.
    pub fn into_payload(self) -> Option<(Payload, AttachmentParser)> {
        match self {
            Self::Text { .. } => None,
            Self::Image { url } => Some((
                Payload::Image(ImageSource::Url(url)),
                AttachmentParser::Image,
            )),
            Self::Audio { data, codec } => Some((
                Payload::Media(MediaSource::Bytes(data), codec),
                AttachmentParser::Audio,
            )),
            Self::Video { url } => Some((
                Payload::Media(MediaSource::Url(url), None),
                AttachmentParser::Video,
            )),
        }
    }
}
#[derive(Clone, Copy)]
struct Limits {
    timeout: Duration,
    max_bytes: usize,
}
#[derive(Clone)]
pub(super) struct Fetcher {
    http: reqwest::Client,
    limits: Limits,
    steps: Arc<[FetchStep]>,
}

impl Fetcher {
    /// The engine chain comes from `[web.fetch] chain`; the registry resolves it
    /// (`Providers::fetch_steps`).
    pub fn from_providers(providers: &Providers, cfg: &WebFetchConfig) -> Result<Self> {
        Ok(Self {
            http: providers.http().clone(),
            limits: Limits {
                timeout: Duration::from_secs(cfg.timeout_secs),
                max_bytes: cfg.max_bytes,
            },
            steps: providers.fetch_steps(cfg)?.into(),
        })
    }

    pub async fn fetch(&self, url: &WebUrl) -> Result<Fetched> {
        let resp = self.send(url).await?;
        let mime = content_type(&resp);
        let body = match Body::of_mime(&mime) {
            Body::Unsupported => match Body::of_extension(url) {
                Some(body) => body,
                None => bail!(AppError::message(format!(
                    "Unsupported content type: {mime}"
                ))),
            },
            body => body,
        };
        match body {
            Body::Image => {
                return Ok(Fetched::Image {
                    url: resp.url().to_string(),
                });
            }
            Body::Audio => {
                let data = self.read_limited(resp).await?;
                let codec = MediaCodec::from_mime(mime.subtype().as_str())
                    .or_else(|| MediaCodec::from_filename(url.path()));
                return Ok(Fetched::Audio {
                    data: data.into(),
                    codec,
                });
            }
            Body::Video => {
                return Ok(Fetched::Video {
                    url: resp.url().to_string(),
                });
            }
            _ => {}
        }

        let text = self.read_text(resp, &mime).await?;
        match body {
            Body::Html => self.extract(url, &text).await,
            Body::Json => Ok(local(pretty_json(&text).unwrap_or(text))),
            Body::Xml => Ok(local(feed::render(&text).unwrap_or(text))),
            _ => Ok(local(text)),
        }
    }

    async fn send(&self, url: &WebUrl) -> Result<reqwest::Response> {
        let resp = self
            .http
            .get(url.as_str())
            .header(reqwest::header::ACCEPT, ACCEPT)
            .timeout(self.limits.timeout)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = truncate_chars(&resp.text().await.unwrap_or_default(), 300);
            bail!(AppError::message(format!("HTTP {status}: {body}")));
        }
        Ok(resp)
    }

    async fn read_text(&self, resp: reqwest::Response, mime: &Mime) -> Result<String> {
        Ok(decode(&self.read_limited(resp).await?, mime))
    }

    /// The bytes of one response, refused over the configured ceiling before they are held.
    async fn read_limited(&self, resp: reqwest::Response) -> Result<bytes::Bytes> {
        let limit = self.limits.max_bytes;
        let too_large = || AppError::message(format!("Response exceeds {limit} bytes"));
        if resp.content_length().is_some_and(|len| len > limit as u64) {
            return Err(too_large());
        }
        let bytes = resp.bytes().await?;
        if bytes.len() > limit {
            return Err(too_large());
        }
        Ok(bytes)
    }

    async fn extract(&self, url: &WebUrl, html: &str) -> Result<Fetched> {
        let mut fallback: Option<(String, FetchEngine)> = None;
        for step in self.steps.iter() {
            let candidate = match step {
                FetchStep::Local => html::render_text(html).ok(),
                FetchStep::Remote { client, capability } => {
                    let call = client.fetch_page(capability, url);
                    match tokio::time::timeout(self.limits.timeout, call).await {
                        Ok(text) => text,
                        Err(_) => {
                            tracing::warn!(
                                url = url.as_str(),
                                timeout_secs = self.limits.timeout.as_secs(),
                                "web fetch provider timed out"
                            );
                            None
                        }
                    }
                }
            };
            let Some(text) = candidate.filter(|text| !text.trim().is_empty()) else {
                continue;
            };
            if usable_text(&text) {
                return Ok(Fetched::Text {
                    text,
                    engine: step.kind(),
                });
            }
            fallback.get_or_insert((text, step.kind()));
        }

        let (text, engine) = fallback.ok_or_else(|| {
            AppError::message(format!("Failed to extract text from {}", url.as_str()))
        })?;
        tracing::warn!(url = url.as_str(), ?engine, "web fetch returned shell text");
        Ok(Fetched::Text { text, engine })
    }
}

fn local(text: String) -> Fetched {
    Fetched::Text {
        text,
        engine: FetchEngine::Local,
    }
}

/// The class a URL's own extension declares, for a server that sends `application/octet-stream` or
/// nothing at all.
impl Body {
    pub(super) fn of_extension(url: &WebUrl) -> Option<Self> {
        let path = url.path();
        if ImageFormat::from_filename(path).is_some() {
            return Some(Self::Image);
        }
        if let Some(codec) = MediaCodec::from_filename(path) {
            return Some(if codec.is_audio() {
                Self::Audio
            } else {
                Self::Video
            });
        }
        match extension_of(path).as_str() {
            "html" | "htm" | "xhtml" => Some(Self::Html),
            "json" => Some(Self::Json),
            "xml" | "rss" | "atom" => Some(Self::Xml),
            "txt" | "md" | "markdown" | "csv" | "log" | "toml" | "yaml" | "yml" => Some(Self::Text),
            _ => None,
        }
    }
}

/// A URL path's extension, lowercased.
fn extension_of(path: &str) -> String {
    path.rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_extension_names_the_class_when_the_server_does_not() {
        for (url, expected) in [
            ("https://example.com/a.png", Some(Body::Image)),
            ("https://example.com/a.mp3", Some(Body::Audio)),
            ("https://example.com/a.mp4", Some(Body::Video)),
            ("https://example.com/a.HTML", Some(Body::Html)),
            ("https://example.com/a.json", Some(Body::Json)),
            ("https://example.com/a.rss", Some(Body::Xml)),
            ("https://example.com/a.md", Some(Body::Text)),
            ("https://example.com/a.bin", None),
            ("https://example.com/page", None),
        ] {
            let url = WebUrl::parse(url).expect("url");
            assert_eq!(Body::of_extension(&url), expected, "{url:?}");
        }
    }
}
