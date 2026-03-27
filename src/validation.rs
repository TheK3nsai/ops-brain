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

#[cfg(test)]
mod tests {
    use super::*;

    // validate_option

    #[test]
    fn validate_option_none_is_ok() {
        assert!(validate_option(None, "severity", INCIDENT_SEVERITIES).is_ok());
    }

    #[test]
    fn validate_option_valid_value() {
        assert!(validate_option(Some("high"), "severity", INCIDENT_SEVERITIES).is_ok());
    }

    #[test]
    fn validate_option_case_insensitive() {
        assert!(validate_option(Some("HIGH"), "severity", INCIDENT_SEVERITIES).is_ok());
        assert!(validate_option(Some("Critical"), "severity", INCIDENT_SEVERITIES).is_ok());
    }

    #[test]
    fn validate_option_invalid_value() {
        let result = validate_option(Some("extreme"), "severity", INCIDENT_SEVERITIES);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("extreme"));
        assert!(msg.contains("severity"));
        assert!(msg.contains("low, medium, high, critical"));
    }

    // validate_required

    #[test]
    fn validate_required_valid() {
        assert!(validate_required("open", "status", INCIDENT_STATUSES).is_ok());
        assert!(validate_required("resolved", "status", INCIDENT_STATUSES).is_ok());
    }

    #[test]
    fn validate_required_case_insensitive() {
        assert!(validate_required("OPEN", "status", INCIDENT_STATUSES).is_ok());
    }

    #[test]
    fn validate_required_invalid() {
        let result = validate_required("archived", "status", INCIDENT_STATUSES);
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
    fn all_runbook_usages_valid() {
        for u in RUNBOOK_USAGES {
            assert!(validate_required(u, "usage", RUNBOOK_USAGES).is_ok());
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
}
