use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Language for AI responses (ISO 639-1 or BCP 47 tag).
/// Default: "en-US". Users can configure via `[ai].language`.
pub type AiLanguage = String;

/// Numeric fingerprint for a directory, sent to AI for analysis.
/// Contains only metadata (paths, sizes, extensions), never file contents.
#[derive(Debug, Clone, Serialize)]
pub struct AiContext {
    pub target_path: String,
    pub size_delta_mb: f64,
    pub current_size_mb: f64,
    pub top_large_files: Vec<(String, u64)>,
    pub primary_extensions: Vec<(String, f32)>,
}

/// Unified AI response for a single path (from JSON parsing).
/// This is the canonical format returned by the AI model for both
/// single-file and batch analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiResponse {
    /// Specific source entity name (e.g. "Docker Desktop Buildx cache")
    pub label_detail: String,
    /// One-sentence description of the directory's purpose
    pub description: String,
    /// "safe" | "low" | "medium" | "high"
    pub risk_level: String,
    /// Governance advice (what to do with this directory)
    pub suggestion: String,
    /// Whether the AI recommends deletion
    pub deletable: bool,
    /// Confidence score 0.0-1.0
    pub confidence: f64,
}

/// Configuration for AI API calls.
/// All fields are always available; only the HTTP call is gated behind `ai` feature.
#[derive(Debug, Clone)]
pub struct AiConfig {
    /// OpenAI-compatible API endpoint URL
    pub api_url: String,
    /// API key
    pub api_key: String,
    /// Model name (e.g. "gpt-4o", "gemini-1.5-flash")
    pub model: String,
    /// Response language (BCP 47 tag, e.g. "en-US", "zh-CN")
    pub language: String,
    /// Max tokens per request (split batch into chunks if exceeded)
    pub max_tokens_per_request: usize,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            api_url: String::new(),
            api_key: String::new(),
            model: "gpt-4o".into(),
            language: "en-US".into(),
            max_tokens_per_request: 4096,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum AiError {
    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("missing field: {0}")]
    MissingField(String),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("AI not configured: missing api_url or api_key")]
    NotConfigured,
    #[error("all retries failed")]
    AllRetriesFailed,
}

/// Build a unified prompt for AI analysis.
/// Supports both single-path and batch analysis (same JSON format).
/// All text fields in the response will be in the specified language.
pub fn build_prompt(contexts: &[AiContext], language: &str) -> String {
    let batch_json = serde_json::to_string_pretty(contexts).unwrap_or_default();

    format!(
        r#"You are a disk cleanup expert for macOS and Linux.
Respond in {language}. All text fields (label_detail, description, suggestion) must be in {language}.

Analyze the following directories. Return a JSON object where keys are directory paths
and values have this exact schema:

{{
  "<target_path>": {{
    "label_detail": "specific source entity name",
    "description": "what this directory is used for",
    "risk_level": "safe|low|medium|high",
    "suggestion": "governance advice",
    "deletable": true,
    "confidence": 0.95
  }}
}}

The "label" field (program category like "build-artifacts", "package-dependencies") is
determined automatically by the program. Do NOT output a label field.
Only output label_detail as the specific source description.

Directories to analyze:
{batch_json}"#,
        language = language,
        batch_json = batch_json,
    )
}

/// Parse an AI JSON response into a path-to-response map.
/// The response JSON must be a single object with path keys and AiResponse values.
/// Returns an empty map if parsing fails.
pub fn try_parse_json(raw: &str) -> HashMap<String, AiResponse> {
    serde_json::from_str::<HashMap<String, AiResponse>>(raw).unwrap_or_default()
}

/// Roughly estimate token count for a prompt string.
/// Uses 3 chars per token as a conservative estimate for mixed CJK/Latin text.
pub fn estimate_tokens(prompt: &str) -> usize {
    let char_count = prompt.chars().count();
    (char_count + 2) / 3
}

// ── HTTP API (gated behind `ai` feature) ─────────────────────────────────

/// Send a prompt to the AI API and return the raw response text.
/// Uses OpenAI Chat Completions format.
#[cfg(feature = "ai")]
pub fn call_ai_api(prompt: &str, config: &AiConfig) -> Result<String, AiError> {
    if config.api_url.is_empty() || config.api_key.is_empty() {
        return Err(AiError::NotConfigured);
    }

    let body = serde_json::json!({
        "model": config.model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": config.max_tokens_per_request,
    });

    let resp = ureq::post(&config.api_url)
        .set("Authorization", &format!("Bearer {}", config.api_key))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| AiError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| AiError::Http(format!("response parse: {e}")))?;

    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| AiError::Http("no content in response".into()))?
        .to_string();

    Ok(content)
}

/// Maximum number of retries for a failed batch.
#[cfg(feature = "ai")]
const MAX_RETRIES: usize = 3;

/// Analyze directories via AI API, with retry and chunking.
///
/// Strategy:
/// 1. Estimate prompt token count. If under limit, send as single batch.
/// 2. If parsing fails, retry same batch up to MAX_RETRIES times.
/// 3. If token count exceeds limit, split into chunks and analyze each.
/// 4. Never fall back to per-path sequential — JSON format is batch-size agnostic.
#[cfg(feature = "ai")]
pub fn analyze(
    contexts: &[AiContext],
    config: &AiConfig,
) -> Result<HashMap<String, AiResponse>, AiError> {
    if contexts.is_empty() {
        return Ok(HashMap::new());
    }

    let estimated = estimate_tokens(&build_prompt(contexts, &config.language));

    if estimated <= config.max_tokens_per_request {
        analyze_batch(contexts, config)
    } else {
        analyze_chunked(contexts, config)
    }
}

/// Send a single batch with retry.
#[cfg(feature = "ai")]
fn analyze_batch(
    contexts: &[AiContext],
    config: &AiConfig,
) -> Result<HashMap<String, AiResponse>, AiError> {
    let prompt = build_prompt(contexts, &config.language);

    for attempt in 0..MAX_RETRIES {
        let raw = call_ai_api(&prompt, config)?;
        let result = try_parse_json(&raw);
        if !result.is_empty() {
            // Verify all expected paths are present
            let all_found = contexts
                .iter()
                .all(|ctx| result.contains_key(&ctx.target_path));
            if all_found {
                return Ok(result);
            }
            // Partial result on last attempt: return what we got
            if attempt == MAX_RETRIES - 1 {
                return Ok(result);
            }
        }
        // Empty or partial: retry
    }

    Err(AiError::AllRetriesFailed)
}

/// Split contexts into token-safe chunks and analyze each.
#[cfg(feature = "ai")]
fn analyze_chunked(
    contexts: &[AiContext],
    config: &AiConfig,
) -> Result<HashMap<String, AiResponse>, AiError> {
    let mut results = HashMap::new();
    let mut remaining = contexts;

    while !remaining.is_empty() {
        // Find the largest prefix that fits within token limit
        let split = find_chunk_boundary(remaining, config);
        let chunk = &remaining[..split];

        let chunk_result = analyze_batch(chunk, config)?;
        results.extend(chunk_result);
        remaining = &remaining[split..];
    }

    Ok(results)
}

/// Find how many contexts fit in one chunk without exceeding token limit.
#[cfg(feature = "ai")]
fn find_chunk_boundary(contexts: &[AiContext], config: &AiConfig) -> usize {
    let mut high = contexts.len();
    let mut low = 1;

    while low < high {
        let mid = (low + high + 1) / 2;
        let prompt = build_prompt(&contexts[..mid], &config.language);
        if estimate_tokens(&prompt) <= config.max_tokens_per_request {
            low = mid;
        } else {
            high = mid - 1;
        }
    }

    low
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_context() -> AiContext {
        AiContext {
            target_path: "/var/log/nginx/".into(),
            size_delta_mb: 4190.0,
            current_size_mb: 4200.0,
            top_large_files: vec![
                ("access.log".into(), 4_100_000_000),
                ("error.log".into(), 90_000_000),
            ],
            primary_extensions: vec![(".log".into(), 100.0)],
        }
    }

    #[test]
    fn test_build_prompt_single() {
        let ctx = sample_context();
        let prompt = build_prompt(&[ctx], "en-US");
        assert!(prompt.contains("/var/log/nginx/"));
        assert!(prompt.contains("en-US"));
        assert!(prompt.contains("label_detail"));
        assert!(prompt.contains("risk_level"));
        assert!(prompt.contains("\"label\" field (program category"));
    }

    #[test]
    fn test_build_prompt_chinese() {
        let ctx = sample_context();
        let prompt = build_prompt(&[ctx], "zh-CN");
        assert!(prompt.contains("zh-CN"));
    }

    #[test]
    fn test_build_prompt_batch() {
        let ctx1 = sample_context();
        let ctx2 = AiContext {
            target_path: "~/Library/Caches/pip/".into(),
            size_delta_mb: 800.0,
            current_size_mb: 2400.0,
            top_large_files: vec![("wheels/*.whl".into(), 2_000_000_000)],
            primary_extensions: vec![(".whl".into(), 80.0), (".gz".into(), 20.0)],
        };
        let prompt = build_prompt(&[ctx1, ctx2], "en-US");
        assert!(prompt.contains("/var/log/nginx/"));
        assert!(prompt.contains("~/Library/Caches/pip/"));
    }

    #[test]
    fn test_try_parse_json_single() {
        let raw = r#"{
            "/var/log/nginx/": {
                "label_detail": "Nginx access logs",
                "description": "HTTP server access and error logs",
                "risk_level": "safe",
                "suggestion": "Configure log rotation or use fail2ban",
                "deletable": true,
                "confidence": 0.95
            }
        }"#;
        let map = try_parse_json(raw);
        assert_eq!(map.len(), 1);
        let resp = map.get("/var/log/nginx/").unwrap();
        assert_eq!(resp.label_detail, "Nginx access logs");
        assert_eq!(resp.risk_level, "safe");
        assert!(resp.deletable);
        assert!((resp.confidence - 0.95).abs() < 0.01);
    }

    #[test]
    fn test_try_parse_json_batch() {
        let raw = r#"{
            "/var/log/nginx/": {
                "label_detail": "Nginx logs",
                "description": "HTTP logs",
                "risk_level": "safe",
                "suggestion": "Rotate logs",
                "deletable": true,
                "confidence": 0.9
            },
            "~/Library/Caches/pip/": {
                "label_detail": "pip wheel cache",
                "description": "Python package build cache",
                "risk_level": "safe",
                "suggestion": "Safe to delete",
                "deletable": true,
                "confidence": 0.85
            }
        }"#;
        let map = try_parse_json(raw);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_try_parse_json_invalid_returns_empty() {
        let map = try_parse_json("not json");
        assert!(map.is_empty());
    }

    #[test]
    fn test_try_parse_json_empty_object() {
        let map = try_parse_json("{}");
        assert!(map.is_empty());
    }

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_short() {
        let n = estimate_tokens("hello world");
        assert!(n > 0);
    }

    #[test]
    fn test_estimate_tokens_cjk() {
        let n = estimate_tokens("你好世界");
        assert!(n > 0);
    }
}
