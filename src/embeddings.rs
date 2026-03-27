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
// Title is repeated to boost its weight in the embedding vector — semantic
// search will more strongly match queries that align with the title.

pub fn prepare_runbook_text(r: &Runbook) -> String {
    let mut text = format!("{}\n{}\n\n{}", r.title, r.title, r.content);
    if let Some(notes) = &r.notes {
        text.push_str("\n\n");
        text.push_str(notes);
    }
    text
}

pub fn prepare_knowledge_text(k: &Knowledge) -> String {
    format!("{}\n{}\n\n{}", k.title, k.title, k.content)
}

pub fn prepare_incident_text(i: &Incident) -> String {
    let mut text = format!("{}\n{}", i.title, i.title);
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
    format!("{}\n{}\n\n{}", h.title, h.title, h.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn prepare_runbook_text_with_notes() {
        let runbook = Runbook {
            id: Uuid::now_v7(),
            title: "Reset AD Password".to_string(),
            slug: "reset-ad-password".to_string(),
            category: Some("active-directory".to_string()),
            content: "Step 1: Open ADUC\nStep 2: Find user".to_string(),
            version: 1,
            tags: vec!["ad".to_string()],
            estimated_minutes: Some(5),
            requires_reboot: false,
            notes: Some("Use RSAT tools on RDS server".to_string()),
            client_id: None,
            cross_client_safe: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let text = prepare_runbook_text(&runbook);
        assert!(text.starts_with("Reset AD Password"));
        assert!(text.contains("Step 1: Open ADUC"));
        assert!(text.contains("Use RSAT tools on RDS server"));
    }

    #[test]
    fn prepare_runbook_text_without_notes() {
        let runbook = Runbook {
            id: Uuid::now_v7(),
            title: "Title".to_string(),
            slug: "title".to_string(),
            category: None,
            content: "Content".to_string(),
            version: 1,
            tags: vec![],
            estimated_minutes: None,
            requires_reboot: false,
            notes: None,
            client_id: None,
            cross_client_safe: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let text = prepare_runbook_text(&runbook);
        assert_eq!(text, "Title\nTitle\n\nContent");
    }

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
    fn prepare_incident_text_full() {
        let incident = Incident {
            id: Uuid::now_v7(),
            title: "Server Outage".to_string(),
            status: "resolved".to_string(),
            severity: "critical".to_string(),
            client_id: None,
            reported_at: Utc::now(),
            resolved_at: Some(Utc::now()),
            symptoms: Some("Cannot RDP".to_string()),
            root_cause: Some("Disk full".to_string()),
            resolution: Some("Cleared temp files".to_string()),
            prevention: Some("Set up disk monitoring".to_string()),
            time_to_resolve_minutes: Some(45),
            notes: None,
            cross_client_safe: false,
            source: None,
            recurrence_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let text = prepare_incident_text(&incident);
        assert!(text.starts_with("Server Outage"));
        assert!(text.contains("Symptoms: Cannot RDP"));
        assert!(text.contains("Root Cause: Disk full"));
        assert!(text.contains("Resolution: Cleared temp files"));
        assert!(text.contains("Prevention: Set up disk monitoring"));
    }

    #[test]
    fn prepare_incident_text_minimal() {
        let incident = Incident {
            id: Uuid::now_v7(),
            title: "Minor Issue".to_string(),
            status: "open".to_string(),
            severity: "low".to_string(),
            client_id: None,
            reported_at: Utc::now(),
            resolved_at: None,
            symptoms: None,
            root_cause: None,
            resolution: None,
            prevention: None,
            time_to_resolve_minutes: None,
            notes: None,
            cross_client_safe: false,
            source: None,
            recurrence_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let text = prepare_incident_text(&incident);
        assert_eq!(text, "Minor Issue\nMinor Issue");
    }

    #[test]
    fn prepare_handoff_text_format() {
        let handoff = Handoff {
            id: Uuid::now_v7(),
            from_session_id: None,
            from_machine: "stealth".to_string(),
            to_machine: Some("cloudlab".to_string()),
            status: "pending".to_string(),
            priority: "high".to_string(),
            title: "Continue DNS migration".to_string(),
            body: "Need to update remaining A records".to_string(),
            context: None,
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
