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

#[derive(thiserror::Error, Debug)]
pub enum AiError {
    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("missing field: {0}")]
    MissingField(String),
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
///
/// Retry strategy (caller responsibility):
/// - On empty map, retry the same batch request (up to 2-3 times).
/// - Do NOT fall back to per-path sequential requests — the JSON format is the same
///   regardless of batch size, so sequential would have the same failure rate.
/// - If all retries fail, report the error to the user.
/// - The only reason to split a batch is token overflow (chunked by token count).
pub fn try_parse_json(raw: &str) -> HashMap<String, AiResponse> {
    serde_json::from_str::<HashMap<String, AiResponse>>(raw).unwrap_or_default()
}

/// Roughly estimate token count for a prompt string.
/// Uses 3 chars per token as a conservative estimate for mixed CJK/Latin text.
pub fn estimate_tokens(prompt: &str) -> usize {
    let char_count = prompt.chars().count();
    (char_count + 2) / 3
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
