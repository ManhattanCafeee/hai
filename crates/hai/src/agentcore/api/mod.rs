use std::io::Cursor;

use base64::{Engine as _, prelude::BASE64_STANDARD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    agentcore::{model_kind::ModelTarget, url::join_url},
    error::{AppError, Result},
};

pub(crate) mod embedding;
pub(crate) mod rerank;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl(pub String);

/// The chat-completions envelope the tooling calls read.
#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

impl ChatResponse {
    pub fn text(&self) -> Option<&str> {
        self.choices.first()?.message.content.as_deref()
    }

    fn image_url(&self) -> Option<&str> {
        self.choices
            .first()?
            .message
            .images
            .first()
            .map(|image| image.image_url.url.as_str())
    }
}

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    message: Message,
}

#[derive(Debug, Default, Deserialize)]
struct Message {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    images: Vec<GeneratedImage>,
}

#[derive(Debug, Deserialize)]
struct GeneratedImage {
    image_url: GeneratedImageUrl,
}

#[derive(Debug, Deserialize)]
struct GeneratedImageUrl {
    url: String,
}

/// The embedding endpoint's response: `{ "data": [{ "embedding": [...] }] }`.
#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    #[serde(default)]
    data: Vec<Embedding>,
}

#[derive(Debug, Deserialize)]
struct Embedding {
    embedding: Vec<f32>,
}

#[derive(Serialize)]
struct RerankRequest<'a> {
    model: &'a str,
    query: &'a str,
    documents: &'a [&'a str],
    top_n: usize,
}

#[derive(Deserialize)]
struct RerankResponse {
    #[serde(default)]
    results: Vec<RerankItem>,
}

#[derive(Deserialize)]
struct RerankItem {
    index: usize,
    relevance_score: f64,
}

/// A document the reranker did not return ranks below every document it did.
fn parse_rerank(response: RerankResponse, count: usize) -> Result<Vec<f64>> {
    let mut scores = vec![f64::NEG_INFINITY; count];
    for item in response.results {
        if let Some(slot) = scores.get_mut(item.index) {
            *slot = item.relevance_score;
        }
    }
    Ok(scores)
}

/// Stateless HTTP client: one instance serves every provider, with the connection parameters
/// passed per call in `ModelTarget`.
#[derive(Debug, Clone)]
pub struct ApiClient {
    http: reqwest::Client,
}

impl ApiClient {
    /// A client that failed to build (a TLS backend the platform cannot initialize) is a startup
    /// error, not a panic: the caller is assembly, which reports it like any other missing piece.
    pub fn try_new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(std::time::Duration::from_secs(30))
            // The rerank call narrows this per request.
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| AppError::message(format!("failed to build the HTTP client: {e}")))?;
        Ok(Self { http })
    }

    async fn request_json<T: DeserializeOwned>(
        &self,
        target: &ModelTarget,
        path: &str,
        body: Value,
        timeout: Option<std::time::Duration>,
    ) -> Result<T> {
        let mut request = self.post(target, path, &body);
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let resp = Self::ensure_success(request.send().await?, "API request").await?;
        Ok(serde_json::from_slice(&resp.bytes().await?)?)
    }

    /// The rerank kind's target is the whole endpoint, so no path is joined onto it.
    pub async fn rerank(
        &self,
        target: &ModelTarget,
        query: &str,
        candidates: &[&str],
        timeout: std::time::Duration,
    ) -> Result<Vec<f64>> {
        let request = RerankRequest {
            model: &target.model,
            query,
            documents: candidates,
            top_n: candidates.len(),
        };
        let response: RerankResponse = self
            .request_json(target, "", serde_json::to_value(request)?, Some(timeout))
            .await?;
        parse_rerank(response, candidates.len())
    }

    /// One request on a model kind: `path` is the part the calling method owns, the provider's own
    /// headers ride along, and the credential header replaces any same-named one they carry.
    fn post(&self, target: &ModelTarget, path: &str, body: &Value) -> reqwest::RequestBuilder {
        let (credential_header, credential_value) = target.credential.header();
        // `RequestBuilder::header` appends, so a provider header under the same name has to be
        // dropped first or the request would carry two credentials.
        let mut headers = (*target.headers).clone();
        headers.remove(credential_header);
        headers.remove("content-type");

        self.http
            .post(join_url(&[&target.base_url, path]))
            .header("Content-Type", "application/json")
            .headers(headers)
            .header(credential_header, credential_value)
            .json(body)
    }

    async fn request_bytes(
        &self,
        target: &ModelTarget,
        path: &str,
        body: Value,
    ) -> Result<(Vec<u8>, String)> {
        let resp =
            Self::ensure_success(self.post(target, path, &body).send().await?, "API request")
                .await?;
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = resp.bytes().await?.to_vec();
        Ok((bytes, content_type))
    }

    pub async fn complete(&self, target: &ModelTarget, content: Value) -> Result<ChatResponse> {
        let body = serde_json::json!({
            "model": &target.model,
            "messages": [{"role": "user", "content": content}],
        });
        self.request_json(target, "chat/completions", body, None)
            .await
    }

    pub async fn embed(&self, target: &ModelTarget, input: &str) -> Result<Vec<f32>> {
        let body = serde_json::json!({ "model": &target.model, "input": input });
        let response: EmbeddingResponse =
            self.request_json(target, "embeddings", body, None).await?;
        if response.data.is_empty() {
            return Err(AppError::message(format!(
                "The embedding response carried no data: {response:?}"
            )));
        }
        let embedding = response
            .data
            .into_iter()
            .next()
            .expect("a non-empty embedding list has a first element");
        Ok(embedding.embedding)
    }

    pub async fn generate_image(
        &self,
        target: &ModelTarget,
        prompt: &str,
        images: &[ImageUrl],
    ) -> Result<Vec<u8>> {
        let mut content = Vec::with_capacity(images.len() + 1);
        for ImageUrl(url) in images {
            content.push(serde_json::json!({"type": "image_url", "image_url": {"url": url}}));
        }
        content.push(serde_json::json!({"type": "text", "text": prompt}));
        let body = serde_json::json!({
            "model": &target.model,
            "modalities": ["image", "text"],
            "messages": [{"role": "user", "content": content}],
        });
        let response: ChatResponse = self
            .request_json(target, "chat/completions", body, None)
            .await?;
        let url = response.image_url().ok_or_else(|| {
            AppError::message(format!(
                "Image generation response has no image URL: {response:?}"
            ))
        })?;
        self.fetch_image(&ImageUrl(url.to_owned())).await
    }

    async fn fetch_image(&self, image: &ImageUrl) -> Result<Vec<u8>> {
        let ImageUrl(url) = image;
        if let Some(rest) = url.strip_prefix("data:") {
            decode_data_url(rest)
        } else {
            let resp =
                Self::ensure_success(self.http.get(url).send().await?, "Image download").await?;
            Ok(resp.bytes().await?.to_vec())
        }
    }

    /// Turns a non-2xx response into the error every tooling endpoint reports, body included.
    async fn ensure_success(resp: reqwest::Response, what: &str) -> Result<reqwest::Response> {
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::message(format!(
                "{what} failed (HTTP {status}): {text}"
            )));
        }
        Ok(resp)
    }

    pub async fn speech(
        &self,
        target: &ModelTarget,
        input: &str,
        voice: &str,
        speed: f32,
    ) -> Result<Vec<u8>> {
        let body = serde_json::json!({
            "model": &target.model,
            "input": input,
            "voice": voice,
            "speed": speed.clamp(0.25, 4.0),
        });
        let (bytes, content_type) = self.request_bytes(target, "audio/speech", body).await?;
        if content_type.contains("pcm") || content_type.contains("L8") {
            Ok(pcm_to_wav(&bytes))
        } else {
            Ok(bytes)
        }
    }
}

fn pcm_to_wav(pcm: &[u8]) -> Vec<u8> {
    let sample_rate = 24000u32;
    let bits_per_sample = 16;
    let channels = 1;
    let samples: Vec<i16> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect();
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Cursor::new(Vec::with_capacity(44 + pcm.len()));
    let mut writer = hound::WavWriter::new(&mut buf, spec).unwrap();
    for s in samples {
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();
    buf.into_inner()
}

fn decode_data_url(rest: &str) -> Result<Vec<u8>> {
    let payload = rest
        .split_once(',')
        .map(|(_, p)| p)
        .ok_or_else(|| AppError::message("invalid image data URL".to_string()))?;
    BASE64_STANDARD
        .decode(payload)
        .map_err(|e| AppError::message(format!("b64 decode failed: {e}")))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use reqwest::header::HeaderValue;

    use super::*;
    use crate::agentcore::model_kind::Credential;

    fn target(credential: Credential, headers: &[(&str, &str)]) -> ModelTarget {
        let mut map = reqwest::header::HeaderMap::new();
        for (name, value) in headers {
            map.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        ModelTarget {
            base_url: "http://127.0.0.1:9/v1".to_owned(),
            model: "m".to_owned(),
            credential,
            headers: Arc::new(map),
        }
    }

    #[test]
    fn the_credential_replaces_a_provider_header_of_the_same_name() {
        let client = ApiClient::try_new().expect("a client for the test");
        let target = target(
            Credential::new("key".to_owned()),
            &[("authorization", "Bearer gateway")],
        );
        let request = client
            .post(&target, "", &serde_json::json!({}))
            .build()
            .unwrap();

        let values: Vec<&HeaderValue> = request.headers().get_all("authorization").iter().collect();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], "Bearer key");
    }

    /// The provider's own headers otherwise ride on the request.
    #[test]
    fn a_provider_header_rides_on_the_request() {
        let client = ApiClient::try_new().expect("a client for the test");
        let target = target(Credential::new("key".to_owned()), &[("x-title", "hai")]);
        let request = client
            .post(&target, "embeddings", &serde_json::json!({}))
            .build()
            .unwrap();

        assert_eq!(request.headers().get("x-title").unwrap(), "hai");
        assert_eq!(request.url().as_str(), "http://127.0.0.1:9/v1/embeddings");
    }

    fn response(body: &str) -> RerankResponse {
        serde_json::from_str(body).expect("valid json")
    }

    #[test]
    fn the_body_asks_for_every_candidate() {
        let request = RerankRequest {
            model: "cohere/rerank-4-fast",
            query: "retry",
            documents: &["a", "b"],
            top_n: 2,
        };
        let body = serde_json::to_value(request).unwrap();
        assert_eq!(body["top_n"], 2);
        assert_eq!(body["documents"], serde_json::json!(["a", "b"]));
        assert_eq!(body["query"], "retry");
    }

    #[test]
    fn scores_follow_the_returned_indexes() {
        let body = response(
            r#"{"results": [{"index": 1, "relevance_score": 0.7}, {"index": 0, "relevance_score": -2.5}]}"#,
        );
        assert_eq!(parse_rerank(body, 2).unwrap(), [-2.5, 0.7]);
    }

    #[test]
    fn a_document_the_reranker_dropped_ranks_last() {
        let body = response(r#"{"results": [{"index": 0, "relevance_score": 0.2}]}"#);
        assert_eq!(parse_rerank(body, 2).unwrap(), [0.2, f64::NEG_INFINITY]);
    }

    #[test]
    fn an_out_of_range_index_is_ignored() {
        let body = response(r#"{"results": [{"index": 7, "relevance_score": 0.2}]}"#);
        assert_eq!(parse_rerank(body, 1).unwrap(), [f64::NEG_INFINITY]);
    }
}
