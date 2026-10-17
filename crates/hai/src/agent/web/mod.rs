//! Web capability: HTTP fetch (the `read` tool) and web search (the `web_search` tool). Stateless
//! and shared across chats.

mod fetch;
mod html;
mod provider;
mod search;

pub use fetch::Fetched;
pub use search::{Recency, SearchRequest, SearchResponse};
use url::Url;

use crate::{
    config::schema::WebConfig,
    error::{AppError, Result},
};

/// An absolute http/https URL, validated at construction; the common input of both capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUrl(Url);

impl WebUrl {
    pub fn parse(raw: &str) -> Result<Self> {
        let url =
            Url::parse(raw).map_err(|e| AppError::message(format!("Invalid URL: {raw}: {e}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(AppError::message(format!(
                "Unsupported URL scheme: {}",
                url.scheme()
            )));
        }
        Ok(Self(url))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The URL's path, for the extension a server's content type does not state.
    pub fn path(&self) -> &str {
        self.0.path()
    }

    /// The URL itself, for a caller that hands it on instead of fetching it.
    pub fn into_url(self) -> Url {
        self.0
    }
}

#[derive(Clone)]
pub struct WebService {
    fetcher: fetch::Fetcher,
    search: search::SearchChain,
}

impl WebService {
    pub fn from_config(cfg: &WebConfig) -> Result<Self> {
        let providers = provider::Providers::from_config(cfg, &|name| std::env::var(name).ok())?;
        Ok(Self {
            search: search::SearchChain::from_providers(&providers, &cfg.search)?,
            fetcher: fetch::Fetcher::from_providers(&providers, &cfg.fetch)?,
        })
    }

    pub async fn fetch(&self, url: &WebUrl) -> Result<Fetched> {
        self.fetcher.fetch(url).await
    }

    pub fn search_available(&self) -> bool {
        self.search.available()
    }

    pub async fn search(&self, req: &SearchRequest) -> Result<SearchResponse> {
        self.search.search(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_url_rejects_non_http_schemes() {
        assert!(WebUrl::parse("ftp://host/x").is_err());
        assert!(WebUrl::parse("not a url").is_err());
        assert_eq!(
            WebUrl::parse("HTTP://Example.COM/a")
                .expect("http url")
                .as_str(),
            "http://example.com/a"
        );
    }
}
