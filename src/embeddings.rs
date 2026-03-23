use serde::{Deserialize, Serialize};

use crate::models::handoff::Handoff;
use crate::models::incident::Incident;
use crate::models::knowledge::Knowledge;
use crate::models::runbook::Runbook;

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

pub fn prepare_runbook_text(r: &Runbook) -> String {
    let mut text = format!("{}\n\n{}", r.title, r.content);
    if let Some(notes) = &r.notes {
        text.push_str("\n\n");
        text.push_str(notes);
    }
    text
}

pub fn prepare_knowledge_text(k: &Knowledge) -> String {
    format!("{}\n\n{}", k.title, k.content)
}

pub fn prepare_incident_text(i: &Incident) -> String {
    let mut text = i.title.clone();
    if let Some(symptoms) = &i.symptoms {
        text.push_str("\n\nSymptoms: ");
        text.push_str(symptoms);
    }
    if let Some(root_cause) = &i.root_cause {
        text.push_str("\n\nRoot Cause: ");
        text.push_str(root_cause);
    }
    if let Some(resolution) = &i.resolution {
        text.push_str("\n\nResolution: ");
        text.push_str(resolution);
    }
    if let Some(prevention) = &i.prevention {
        text.push_str("\n\nPrevention: ");
        text.push_str(prevention);
    }
    text
}

pub fn prepare_handoff_text(h: &Handoff) -> String {
    format!("{}\n\n{}", h.title, h.body)
}
