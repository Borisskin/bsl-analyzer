use crate::error::{EmbeddingFailure, EmbeddingFailureCode, SearchError};
use crate::ports::EmbeddingGenerator;
use serde::{Deserialize, Serialize};
use std::ops::Range;

#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    pub base_url: String,
    pub model: String,
    pub dim: Option<usize>,
    pub api_key: Option<String>,
    pub provider: Option<String>,
    pub max_request_bytes: usize,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434".to_owned(),
            model: "qwen3-embedding".to_owned(),
            dim: Some(1024),
            api_key: None,
            provider: None,
            max_request_bytes: Self::DEFAULT_MAX_REQUEST_BYTES,
        }
    }
}

impl EmbedderConfig {
    pub const DEFAULT_MAX_REQUEST_BYTES: usize = 1_048_576;

    pub fn request_bytes_from_env() -> Result<usize, SearchError> {
        match std::env::var("EMBEDDING_MAX_REQUEST_BYTES") {
            Ok(value) => Self::parse_request_bytes(Some(&value)),
            Err(std::env::VarError::NotPresent) => Self::parse_request_bytes(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(invalid_config()),
        }
    }

    fn parse_request_bytes(value: Option<&str>) -> Result<usize, SearchError> {
        match value {
            None => Ok(Self::DEFAULT_MAX_REQUEST_BYTES),
            Some(value) => {
                value.parse::<usize>().ok().filter(|n| *n > 0).ok_or_else(invalid_config)
            }
        }
    }

    pub fn validate(&self) -> Result<(), SearchError> {
        if self.max_request_bytes == 0 {
            Err(invalid_config())
        } else {
            Ok(())
        }
    }
}

fn invalid_config() -> SearchError {
    EmbeddingFailure::new(EmbeddingFailureCode::EmbeddingInvalidConfig).into()
}

pub struct Embedder {
    config: EmbedderConfig,
    /// Resilient agent for the unattended batch indexing pass: a long global timeout, paired
    /// with [`Self::MAX_RETRIES`] in [`Self::embed_batch`].
    agent: ureq::Agent,
    /// Tight agent for interactive single-query embeds ([`Self::embed`]). A `search_code` caller
    /// is waiting and the engine mutex is held across the call, so the query embed must fail
    /// fast instead of inheriting the batch path's minutes-long timeout-and-retry budget.
    interactive_agent: ureq::Agent,
}

impl Clone for Embedder {
    fn clone(&self) -> Self {
        Self::new(self.config.clone())
    }
}

impl Embedder {
    /// Global timeout for an interactive query embed. Bounds how long [`Self::embed`] can hold
    /// the engine mutex, so one slow embed cannot stall every concurrent `search_code`.
    const INTERACTIVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);

    pub fn new(config: EmbedderConfig) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .build()
            .new_agent();
        let interactive_agent = ureq::Agent::config_builder()
            .timeout_global(Some(Self::INTERACTIVE_TIMEOUT))
            .build()
            .new_agent();
        Self { config, agent, interactive_agent }
    }

    pub fn dim(&self) -> usize {
        self.config.dim.unwrap_or(1024)
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// A clone of this embedder's configuration, so a caller can rebuild a standalone embedder
    /// (e.g. the off-lock overlay warmup) without reaching into private fields.
    pub fn config(&self) -> EmbedderConfig {
        self.config.clone()
    }

    const MAX_RETRIES: u32 = 10;

    /// Plan caller-visible requests so each existing owner retains its checkpoints.
    pub fn batch_ranges(
        &self,
        texts: &[&str],
        max_items: usize,
    ) -> Result<Vec<Range<usize>>, SearchError> {
        self.config.validate()?;
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let envelope = self.serialize_request(&[])?.len();
        let mut ranges = Vec::new();
        let mut start = 0;
        let mut bytes = envelope;
        for (i, text) in texts.iter().enumerate() {
            let item = serde_json::to_vec(text)
                .map_err(|_| failure(EmbeddingFailureCode::EmbeddingFailed))?
                .len();
            let singleton = envelope
                .checked_add(item)
                .ok_or_else(|| failure(EmbeddingFailureCode::EmbeddingInputTooLarge))?;
            if singleton > self.config.max_request_bytes {
                return Err(self.size_failure(singleton, true));
            }
            let next = bytes.checked_add(item).and_then(|n| n.checked_add(usize::from(i > start)));
            if i - start == max_items.max(1)
                || next.is_none_or(|n| n > self.config.max_request_bytes)
            {
                ranges.push(start..i);
                start = i;
                bytes = singleton;
            } else {
                bytes = next.expect("checked above");
            }
        }
        ranges.push(start..texts.len());
        Ok(ranges)
    }

    /// One bounded HTTP request. Work-set owners call `batch_ranges` before scheduling.
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, SearchError> {
        let body = self.prepare_request(texts)?;
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        for attempt in 0..Self::MAX_RETRIES {
            match self.send_request(&self.agent, &body, texts.len()) {
                Ok(result) => return Ok(result),
                Err(error) => {
                    if attempt + 1 == Self::MAX_RETRIES
                        || error.embedding_failure().is_some_and(|f| {
                            f.code == EmbeddingFailureCode::EmbeddingRequestTooLarge
                        })
                    {
                        return Err(error);
                    }
                    let delay = std::time::Duration::from_millis(500 * 2u64.pow(attempt.min(6)));
                    tracing::warn!(
                        attempt = attempt + 1,
                        max = Self::MAX_RETRIES,
                        delay_ms = delay.as_millis() as u64,
                        "embedding batch failed, retrying: {error}"
                    );
                    std::thread::sleep(delay);
                }
            }
        }
        unreachable!("the last attempt returns its result")
    }

    fn serialize_request(&self, texts: &[&str]) -> Result<Vec<u8>, SearchError> {
        let provider_only = self.config.provider.as_deref().map(|s| [s]);
        let provider =
            provider_only.as_ref().map(|only| ProviderRouting { only, allow_fallbacks: false });
        serde_json::to_vec(&EmbeddingRequest {
            model: &self.config.model,
            input: texts,
            dimensions: self.config.dim,
            provider,
        })
        .map_err(|_| failure(EmbeddingFailureCode::EmbeddingFailed))
    }

    fn prepare_request(&self, texts: &[&str]) -> Result<Vec<u8>, SearchError> {
        self.config.validate()?;
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let body = self.serialize_request(texts)?;
        if body.len() > self.config.max_request_bytes {
            return Err(self.size_failure(body.len(), texts.len() == 1));
        }
        Ok(body)
    }

    fn size_failure(&self, request_bytes: usize, singleton: bool) -> SearchError {
        EmbeddingFailure {
            code: if singleton {
                EmbeddingFailureCode::EmbeddingInputTooLarge
            } else {
                EmbeddingFailureCode::EmbeddingRequestTooLarge
            },
            request_bytes: Some(request_bytes),
            max_request_bytes: Some(self.config.max_request_bytes),
        }
        .into()
    }

    fn send_request(
        &self,
        agent: &ureq::Agent,
        body: &[u8],
        input_count: usize,
    ) -> Result<Vec<Vec<f32>>, SearchError> {
        let url = format!("{}/v1/embeddings", self.config.base_url);
        let mut req = agent.post(&url).header("Content-Type", "application/json");
        if let Some(ref key) = self.config.api_key {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }
        let mut resp = req.send(body).map_err(|e| transport_failure(e, false))?;
        // Retain ureq's existing 10 MiB read bound, independent of the request ceiling.
        let body = resp.body_mut().read_to_string().map_err(|e| transport_failure(e, true))?;
        let mut data = serde_json::from_str::<EmbeddingResponse>(&body)
            .map_err(|_| failure(EmbeddingFailureCode::EmbeddingInvalidResponse))?
            .data;
        data.sort_by_key(|d| d.index);
        if data.len() != input_count
            || data.iter().enumerate().any(|(index, d)| {
                d.index != index
                    || d.embedding.len() != self.dim()
                    || d.embedding.iter().any(|v| !v.is_finite())
            })
        {
            return Err(failure(EmbeddingFailureCode::EmbeddingInvalidResponse));
        }
        Ok(data.into_iter().map(|d| d.embedding).collect())
    }

    /// Embed a single interactive query, fail-fast. Unlike [`Self::embed_batch`] (the resilient
    /// indexing path), this makes ONE attempt on the tight-timeout [`Self::interactive_agent`]:
    /// the caller is an interactive `search_code` holding the engine mutex, so a stuck embedding
    /// service must surface an error in seconds rather than retry for minutes and block every
    /// concurrent search. A transient failure is the caller's to retry as a whole search.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        let mut results = self.embed_batch_interactive(&[text])?;
        results.pop().ok_or_else(|| failure(EmbeddingFailureCode::EmbeddingInvalidResponse))
    }

    /// Embed a batch fail-fast on the interactive agent. For the workspace-overlay refresh, which
    /// runs while the engine mutex is held (an interactive semantic search or the warmup prime):
    /// it must NOT inherit the indexing path's minutes-long retry budget and stall every
    /// concurrent search. A transient failure just leaves those chunks un-embedded until the next
    /// refresh re-attempts them; lexical search stays available meanwhile.
    pub fn embed_batch_interactive(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, SearchError> {
        let body = self.prepare_request(texts)?;
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.send_request(&self.interactive_agent, &body, texts.len())
    }

    pub fn health_check(&self) -> Result<(), SearchError> {
        self.config.validate()?;
        let health_url = format!("{}/health", self.config.base_url);
        let models_url = format!("{}/v1/models", self.config.base_url);
        if self.agent.get(&health_url).call().is_err() {
            self.agent.get(&models_url).call().map_err(|e| transport_failure(e, false))?;
        }
        Ok(())
    }
}

fn failure(code: EmbeddingFailureCode) -> SearchError {
    EmbeddingFailure::new(code).into()
}

fn transport_failure(error: ureq::Error, reading_response: bool) -> SearchError {
    use EmbeddingFailureCode::*;
    let code = match error {
        ureq::Error::StatusCode(413) => EmbeddingRequestTooLarge,
        ureq::Error::StatusCode(_) => EmbeddingProviderError,
        ureq::Error::Timeout(_) => EmbeddingTimeout,
        ureq::Error::BodyExceedsLimit(_) if reading_response => EmbeddingResponseTooLarge,
        ureq::Error::Io(_)
        | ureq::Error::Http(_)
        | ureq::Error::BadUri(_)
        | ureq::Error::Protocol(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Tls(_)
        | ureq::Error::Rustls(_)
        | ureq::Error::RedirectFailed
        | ureq::Error::TooManyRedirects
        | ureq::Error::InvalidProxyUrl
        | ureq::Error::ConnectProxyFailed(_)
        | ureq::Error::TlsRequired
        | ureq::Error::RequireHttpsOnly(_)
        | ureq::Error::LargeResponseHeader(_, _) => EmbeddingTransportError,
        _ => EmbeddingFailed,
    };
    failure(code)
}

impl EmbeddingGenerator for Embedder {
    fn model_id(&self) -> &str {
        self.model()
    }

    fn dimension(&self) -> usize {
        self.dim()
    }

    fn batch_ranges(
        &self,
        texts: &[&str],
        max_items: usize,
    ) -> Result<Vec<Range<usize>>, SearchError> {
        Self::batch_ranges(self, texts, max_items)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, SearchError> {
        Self::embed_batch(self, texts)
    }
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<ProviderRouting<'a>>,
}

#[derive(Serialize)]
struct ProviderRouting<'a> {
    only: &'a [&'a str],
    allow_fallbacks: bool,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    index: usize,
    embedding: Vec<f32>,
}

#[cfg(test)]
pub(crate) mod payload_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_configuration_defaults_override_and_rejection() {
        assert_eq!(EmbedderConfig::parse_request_bytes(None).unwrap(), 1_048_576);
        assert_eq!(EmbedderConfig::parse_request_bytes(Some("73")).unwrap(), 73);
        for value in ["0", "", "-1", "not-a-number", "184467440737095516160"] {
            let error = EmbedderConfig::parse_request_bytes(Some(value)).unwrap_err();
            assert_eq!(error.to_string(), "embedding_invalid_config");
        }
        let embedder = Embedder::new(EmbedderConfig { max_request_bytes: 0, ..Default::default() });
        assert_eq!(embedder.embed("input").unwrap_err().to_string(), "embedding_invalid_config");
        assert!(embedder.embed_batch_interactive(&[]).is_err());
    }
}
