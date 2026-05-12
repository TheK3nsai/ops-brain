use serde::{Deserialize, Serialize};

use crate::models::handoff::Handoff;
use crate::models::knowledge::Knowledge;

#[derive(Clone, Debug)]
pub struct EmbeddingClient {
    client: reqwest::Client,
    url: String,
    api_key: Option<String>,
    model: String,
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl EmbeddingClient {
    pub fn new(url: String, model: String, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            api_key,
            model,
        }
    }

    /// Embed a single text string.
    pub async fn embed_text(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let texts = vec![text.to_string()];
        let mut results = self.embed_texts(&texts).await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding response"))
    }

    /// Embed multiple texts in a single API call (batch). Uses OpenAI-compatible endpoint.
    pub async fn embed_texts(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let request = EmbeddingRequest {
            model: &self.model,
            input: texts,
        };

        let mut req = self.client.post(&self.url).json(&request);
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let response = req.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Embedding API error {status}: {body}");
        }

        let resp: EmbeddingResponse = response.json().await?;
        Ok(resp.data.into_iter().map(|d| d.embedding).collect())
    }
}

// ===== TEXT PREPARATION =====
// Each function produces the text that gets embedded for a given record.
// Title is repeated to boost its weight in the embedding vector — semantic
// search will more strongly match queries that align with the title.

/// nomic-embed-text has an 8192-token context window. Real content
/// (markdown, code, JSON) tokenizes at ~1-1.15 chars/token.
/// 6000 chars ≈ ~5200-6000 tokens, leaving ample headroom.
const MAX_EMBEDDING_CHARS: usize = 6_000;

fn truncate_for_embedding(text: String) -> String {
    if text.len() <= MAX_EMBEDDING_CHARS {
        return text;
    }
    // Truncate at a char boundary (floor_char_boundary equivalent)
    let mut end = MAX_EMBEDDING_CHARS;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = text[..end].to_string();
    truncated.push_str("\n\n[truncated for embedding]");
    truncated
}

pub fn prepare_knowledge_text(k: &Knowledge) -> String {
    truncate_for_embedding(format!("{}\n{}\n\n{}", k.title, k.title, k.content))
}

pub fn prepare_handoff_text(h: &Handoff) -> String {
    truncate_for_embedding(format!("{}\n{}\n\n{}", h.title, h.title, h.body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn prepare_knowledge_text_format() {
        let knowledge = Knowledge {
            id: Uuid::now_v7(),
            title: "VPN Setup Guide".to_string(),
            content: "Configure WireGuard tunnel".to_string(),
            category: Some("networking".to_string()),
            tags: vec![],
            client_id: None,
            cross_client_safe: false,
            last_verified_at: None,
            author: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let text = prepare_knowledge_text(&knowledge);
        assert_eq!(
            text,
            "VPN Setup Guide\nVPN Setup Guide\n\nConfigure WireGuard tunnel"
        );
    }

    #[test]
    fn truncate_for_embedding_short_text() {
        let text = "short text".to_string();
        assert_eq!(truncate_for_embedding(text.clone()), text);
    }

    #[test]
    fn truncate_for_embedding_long_text() {
        let text = "x".repeat(30_000);
        let result = truncate_for_embedding(text);
        assert!(result.len() < 30_000);
        assert!(result.ends_with("[truncated for embedding]"));
        // The truncated content should be MAX_EMBEDDING_CHARS worth of x's
        assert!(result.starts_with("xxxx"));
    }

    #[test]
    fn truncate_for_embedding_multibyte() {
        // Ensure we don't split in the middle of a multi-byte char
        let mut text = "a".repeat(MAX_EMBEDDING_CHARS - 1);
        text.push('é'); // 2-byte char that would straddle the boundary
        text.push_str(&"b".repeat(100));
        let result = truncate_for_embedding(text);
        assert!(result.ends_with("[truncated for embedding]"));
        // Should be valid UTF-8 (would panic on invalid)
        let _ = result.as_str();
    }

    #[test]
    fn prepare_handoff_text_truncates_long_body() {
        let handoff = Handoff {
            id: Uuid::now_v7(),
            from_session_id: None,
            from_agent: "dev-laptop".to_string(),
            to_agent: Some("prod-server".to_string()),
            status: "pending".to_string(),
            priority: "high".to_string(),
            category: "action".to_string(),
            title: "Big handoff".to_string(),
            body: "x".repeat(30_000),
            context: None,
            in_reply_to: None,
            commit_hash: None,
            merge_commit: None,
            merged_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let text = prepare_handoff_text(&handoff);
        assert!(text.len() < 30_000);
        assert!(text.ends_with("[truncated for embedding]"));
        assert!(text.starts_with("Big handoff"));
    }

    #[test]
    fn prepare_handoff_text_format() {
        let handoff = Handoff {
            id: Uuid::now_v7(),
            from_session_id: None,
            from_agent: "dev-laptop".to_string(),
            to_agent: Some("prod-server".to_string()),
            status: "pending".to_string(),
            priority: "high".to_string(),
            category: "action".to_string(),
            title: "Continue DNS migration".to_string(),
            body: "Need to update remaining A records".to_string(),
            context: None,
            in_reply_to: None,
            commit_hash: None,
            merge_commit: None,
            merged_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let text = prepare_handoff_text(&handoff);
        assert_eq!(
            text,
            "Continue DNS migration\nContinue DNS migration\n\nNeed to update remaining A records"
        );
    }
}
