use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Handoff {
    pub id: Uuid,
    pub from_session_id: Option<Uuid>,
    pub from_agent: String,
    pub to_agent: Option<String>,
    pub status: String,
    pub priority: String,
    /// "action" (persistent until completed) or "notify" (ephemeral FYI,
    /// pruned from operational queries after 7 days).
    pub category: String,
    pub title: String,
    pub body: String,
    pub context: Option<serde_json::Value>,
    /// Parent handoff ID when this handoff is a reply. ON DELETE SET NULL
    /// at the database level — replies survive parent deletion.
    pub in_reply_to: Option<Uuid>,
    /// Commit ref set at completion time. Carries the work product
    /// structurally so it can be joined against `mark_merged` later.
    pub commit_hash: Option<String>,
    /// Merge commit set by `mark_merged` once the bundle containing
    /// `commit_hash` lands in main.
    pub merge_commit: Option<String>,
    pub merged_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
