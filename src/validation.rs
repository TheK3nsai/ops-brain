//! Input validation and serde helpers for tool parameters.
//!
//! Validates free-form string fields against known values and returns
//! helpful error messages listing valid options.

use serde::Deserialize;

/// Deserialize an `Option<i64>` that accepts both `50` (number) and `"50"` (string).
///
/// Some MCP clients serialize integers as JSON strings. This deserializer
/// handles both forms gracefully so tools never reject valid limit values.
pub fn deserialize_flexible_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FlexibleI64 {
        Int(i64),
        Str(String),
    }

    let opt: Option<FlexibleI64> = Option::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(FlexibleI64::Int(n)) => Ok(Some(n)),
        Some(FlexibleI64::Str(s)) => s
            .parse::<i64>()
            .map(Some)
            .map_err(|_| D::Error::custom(format!("invalid integer string: \"{s}\""))),
    }
}

/// Deserialize an `Option<i32>` that accepts both `5` (number) and `"5"` (string).
pub fn deserialize_flexible_i32<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FlexibleI32 {
        Int(i32),
        Str(String),
    }

    let opt: Option<FlexibleI32> = Option::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(FlexibleI32::Int(n)) => Ok(Some(n)),
        Some(FlexibleI32::Str(s)) => s
            .parse::<i32>()
            .map(Some)
            .map_err(|_| D::Error::custom(format!("invalid integer string: \"{s}\""))),
    }
}

pub const HANDOFF_STATUSES: &[&str] = &["pending", "accepted", "completed", "merged"];
pub const HANDOFF_PRIORITIES: &[&str] = &["low", "normal", "high", "critical"];
pub const HANDOFF_CATEGORIES: &[&str] = &["action", "notify"];
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

/// Validate an agent identifier. Free-form slug, 1–80 chars, ASCII alphanumeric
/// plus `-`, `_`, `.`. No allowlist, no normalization, no case folding —
/// whatever the caller says it is, it is. Trims surrounding whitespace.
///
/// v2.0 replacement for the v1.x CC-fleet allowlist (CC_TEAM, normalize_machine_name,
/// is_valid_cc_name). Existing values like `CC-Stealth`, `CC-Cloud`, `Codex-HSR`,
/// `Gemini-Stealth`, and older lowercase slugs all pass cleanly. Recommended
/// fleet convention mirrors the CC names: `<Kind>-<Infra>` (`Codex-HSR`,
/// `Gemini-Stealth`) but this is documentation, not enforcement.
pub fn validate_agent_name(input: &str) -> Result<&str, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("agent_name cannot be empty".to_string());
    }
    if trimmed.len() > 80 {
        return Err(format!(
            "agent_name too long ({} chars, max 80)",
            trimmed.len()
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(format!(
            "agent_name '{trimmed}' contains invalid characters \
             (allowed: a-zA-Z0-9 . - _)"
        ));
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // validate_option

    #[test]
    fn validate_option_none_is_ok() {
        assert!(validate_option(None, "priority", HANDOFF_PRIORITIES).is_ok());
    }

    #[test]
    fn validate_option_valid_value() {
        assert!(validate_option(Some("high"), "priority", HANDOFF_PRIORITIES).is_ok());
    }

    #[test]
    fn validate_option_case_insensitive() {
        assert!(validate_option(Some("HIGH"), "priority", HANDOFF_PRIORITIES).is_ok());
        assert!(validate_option(Some("Critical"), "priority", HANDOFF_PRIORITIES).is_ok());
    }

    #[test]
    fn validate_option_invalid_value() {
        let result = validate_option(Some("extreme"), "priority", HANDOFF_PRIORITIES);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("extreme"));
        assert!(msg.contains("priority"));
        assert!(msg.contains("low, normal, high, critical"));
    }

    // validate_required

    #[test]
    fn validate_required_valid() {
        assert!(validate_required("pending", "status", HANDOFF_STATUSES).is_ok());
        assert!(validate_required("completed", "status", HANDOFF_STATUSES).is_ok());
    }

    #[test]
    fn validate_required_case_insensitive() {
        assert!(validate_required("PENDING", "status", HANDOFF_STATUSES).is_ok());
    }

    #[test]
    fn validate_required_invalid() {
        let result = validate_required("archived", "status", HANDOFF_STATUSES);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("archived"));
    }

    // All enum constants are self-consistent

    #[test]
    fn all_handoff_statuses_valid() {
        for s in HANDOFF_STATUSES {
            assert!(validate_required(s, "status", HANDOFF_STATUSES).is_ok());
        }
    }

    #[test]
    fn all_handoff_priorities_valid() {
        for p in HANDOFF_PRIORITIES {
            assert!(validate_required(p, "priority", HANDOFF_PRIORITIES).is_ok());
        }
    }

    #[test]
    fn all_handoff_categories_valid() {
        for c in HANDOFF_CATEGORIES {
            assert!(validate_required(c, "category", HANDOFF_CATEGORIES).is_ok());
        }
    }

    #[test]
    fn all_search_modes_valid() {
        for m in SEARCH_MODES {
            assert!(validate_required(m, "mode", SEARCH_MODES).is_ok());
        }
    }

    #[test]
    fn all_briefing_types_valid() {
        for t in BRIEFING_TYPES {
            assert!(validate_required(t, "type", BRIEFING_TYPES).is_ok());
        }
    }

    // deserialize_flexible_i64

    #[derive(Debug, serde::Deserialize)]
    struct TestLimit {
        #[serde(default, deserialize_with = "super::deserialize_flexible_i64")]
        limit: Option<i64>,
    }

    #[test]
    fn flexible_i64_from_number() {
        let v: TestLimit = serde_json::from_str(r#"{"limit": 50}"#).unwrap();
        assert_eq!(v.limit, Some(50));
    }

    #[test]
    fn flexible_i64_from_string() {
        let v: TestLimit = serde_json::from_str(r#"{"limit": "50"}"#).unwrap();
        assert_eq!(v.limit, Some(50));
    }

    #[test]
    fn flexible_i64_null() {
        let v: TestLimit = serde_json::from_str(r#"{"limit": null}"#).unwrap();
        assert_eq!(v.limit, None);
    }

    #[test]
    fn flexible_i64_missing() {
        let v: TestLimit = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(v.limit, None);
    }

    #[test]
    fn flexible_i64_invalid_string() {
        let result: Result<TestLimit, _> = serde_json::from_str(r#"{"limit": "abc"}"#);
        assert!(result.is_err());
    }

    // validate_agent_name

    #[test]
    fn agent_name_accepts_legacy_cc_values() {
        assert_eq!(validate_agent_name("CC-Stealth").unwrap(), "CC-Stealth");
        assert_eq!(validate_agent_name("CC-Cloud").unwrap(), "CC-Cloud");
        assert_eq!(validate_agent_name("CC-HSR").unwrap(), "CC-HSR");
        assert_eq!(validate_agent_name("CC-CPA").unwrap(), "CC-CPA");
    }

    #[test]
    fn agent_name_accepts_freeform_slugs() {
        assert_eq!(validate_agent_name("Codex-HSR").unwrap(), "Codex-HSR");
        assert_eq!(
            validate_agent_name("Gemini-Stealth").unwrap(),
            "Gemini-Stealth"
        );
        assert_eq!(validate_agent_name("codex-hsr").unwrap(), "codex-hsr");
        assert_eq!(
            validate_agent_name("opencode_local").unwrap(),
            "opencode_local"
        );
        assert_eq!(
            validate_agent_name("com.anthropic.claude").unwrap(),
            "com.anthropic.claude"
        );
        assert_eq!(validate_agent_name("agent-123").unwrap(), "agent-123");
    }

    #[test]
    fn agent_name_trims_whitespace() {
        assert_eq!(validate_agent_name("  CC-Stealth\n").unwrap(), "CC-Stealth");
        assert_eq!(validate_agent_name("\tCodex-HSR ").unwrap(), "Codex-HSR");
    }

    #[test]
    fn agent_name_rejects_empty() {
        assert!(validate_agent_name("").is_err());
        assert!(validate_agent_name("   ").is_err());
        assert!(validate_agent_name("\t\n").is_err());
    }

    #[test]
    fn agent_name_rejects_oversized() {
        let long = "a".repeat(81);
        let err = validate_agent_name(&long).unwrap_err();
        assert!(err.contains("too long"));
        // 80 is the max — exactly 80 should pass.
        let max = "a".repeat(80);
        assert!(validate_agent_name(&max).is_ok());
    }

    #[test]
    fn agent_name_rejects_invalid_chars() {
        assert!(validate_agent_name("agent name").is_err()); // space
        assert!(validate_agent_name("agent/path").is_err()); // slash
        assert!(validate_agent_name("agent@host").is_err()); // at
        assert!(validate_agent_name("agent\nnewline").is_err());
        assert!(validate_agent_name("agent\u{00e9}").is_err()); // non-ASCII
    }
}
