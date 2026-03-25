//! Input validation for tool parameters.
//!
//! Validates free-form string fields against known values and returns
//! helpful error messages listing valid options.

pub const INCIDENT_SEVERITIES: &[&str] = &["low", "medium", "high", "critical"];
pub const INCIDENT_STATUSES: &[&str] = &["open", "resolved"];
pub const HANDOFF_STATUSES: &[&str] = &["pending", "accepted", "completed"];
pub const HANDOFF_PRIORITIES: &[&str] = &["low", "normal", "high", "critical"];
pub const RUNBOOK_USAGES: &[&str] = &["followed", "not-applicable", "not-followed"];
pub const SEARCH_MODES: &[&str] = &["fts", "semantic", "hybrid"];
pub const BRIEFING_TYPES: &[&str] = &["daily", "weekly"];

/// Validate a value against a list of allowed values.
/// Returns Ok(()) if valid or None, Err(message) if invalid.
pub fn validate_option(
    value: Option<&str>,
    field_name: &str,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(v) = value {
        let lower = v.to_lowercase();
        if !allowed.contains(&lower.as_str()) {
            return Err(format!(
                "Invalid {field_name}: '{}'. Valid values: {}",
                v,
                allowed.join(", ")
            ));
        }
    }
    Ok(())
}

/// Validate a required value against a list of allowed values.
pub fn validate_required(value: &str, field_name: &str, allowed: &[&str]) -> Result<(), String> {
    let lower = value.to_lowercase();
    if !allowed.contains(&lower.as_str()) {
        return Err(format!(
            "Invalid {field_name}: '{}'. Valid values: {}",
            value,
            allowed.join(", ")
        ));
    }
    Ok(())
}
