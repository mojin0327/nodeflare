//! Gemini embedding client for semantic `search_tools`.
//!
//! Generates embedding vectors for tool catalog entries (name + description) and for
//! search queries. Entirely optional: if `GEMINI_API_KEY` is unset the client is
//! `None` and the proxy falls back to lexical search, so semantic search degrades
//! gracefully and never blocks a request. Every call returns `None` on any failure
//! for the same reason.

use serde_json::json;

const DEFAULT_MODEL: &str = "gemini-embedding-001";
const DEFAULT_DIMENSIONS: usize = 768;
const ENDPOINT_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

#[derive(Clone)]
pub struct EmbeddingClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
    dimensions: usize,
}

impl EmbeddingClient {
    /// Build from environment. Returns `None` (semantic search disabled) when
    /// `GEMINI_API_KEY` is missing/empty.
    pub fn from_env(http: reqwest::Client) -> Option<Self> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())?;
        let model = std::env::var("EMBEDDING_MODEL")
            .ok()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let dimensions = std::env::var("EMBEDDING_DIMENSIONS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_DIMENSIONS);
        tracing::info!("semantic search enabled: model={}, dim={}", model, dimensions);
        Some(Self {
            http,
            api_key,
            model,
            dimensions,
        })
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Embed a single text (e.g. a search query). `None` on any failure.
    pub async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        // Pass the API key as a request header, not a URL query parameter,
        // to avoid it appearing in access logs and HTTP tracing output.
        let url = format!("{}/{}:embedContent", ENDPOINT_BASE, self.model);
        let body = json!({
            "model": format!("models/{}", self.model),
            "content": { "parts": [{ "text": text }] },
            "outputDimensionality": self.dimensions,
        });

        let resp = match self.http.post(&url).header("x-goog-api-key", &self.api_key).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("gemini embed request failed: {}", e);
                return None;
            }
        };
        if !resp.status().is_success() {
            tracing::warn!("gemini embed: HTTP {}", resp.status());
            return None;
        }
        let v: serde_json::Value = resp.json().await.ok()?;
        self.parse_values(v.get("embedding")?)
    }

    /// Embed many texts in one request. Returns a vector aligned with `texts`; entries
    /// that fail to parse are `None`. Returns `None` for the whole call on transport or
    /// HTTP failure (caller can retry later).
    pub async fn embed_batch(&self, texts: &[String]) -> Option<Vec<Option<Vec<f32>>>> {
        if texts.is_empty() {
            return Some(Vec::new());
        }
        let url = format!("{}/{}:batchEmbedContents", ENDPOINT_BASE, self.model);
        let requests: Vec<serde_json::Value> = texts
            .iter()
            .map(|t| {
                json!({
                    "model": format!("models/{}", self.model),
                    "content": { "parts": [{ "text": t }] },
                    "outputDimensionality": self.dimensions,
                })
            })
            .collect();
        let body = json!({ "requests": requests });

        let resp = match self.http.post(&url).header("x-goog-api-key", &self.api_key).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("gemini batch embed request failed: {}", e);
                return None;
            }
        };
        if !resp.status().is_success() {
            tracing::warn!("gemini batch embed: HTTP {}", resp.status());
            return None;
        }
        let v: serde_json::Value = resp.json().await.ok()?;
        let embeddings = v.get("embeddings")?.as_array()?;
        Some(embeddings.iter().map(|e| self.parse_values(e)).collect())
    }

    /// Extract a `values` float array from an `{ "values": [...] }` node and validate
    /// its dimension matches the configured column width.
    fn parse_values(&self, node: &serde_json::Value) -> Option<Vec<f32>> {
        let arr = node.get("values")?.as_array()?;
        let out: Vec<f32> = arr.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
        if out.len() == self.dimensions {
            Some(out)
        } else {
            tracing::warn!(
                "gemini embed dimension mismatch: got {}, expected {}",
                out.len(),
                self.dimensions
            );
            None
        }
    }
}
