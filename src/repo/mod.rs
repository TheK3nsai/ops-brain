/// Build an OR-joined tsquery string for FTS fallback.
/// Returns None if query has <2 words (OR is pointless with 0-1 words).
/// Used when AND-based websearch_to_tsquery returns zero results.
pub(crate) fn build_or_tsquery_text(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '-')
                .collect::<String>()
        })
        .filter(|w| !w.is_empty() && w != "-")
        .collect();

    if words.len() < 2 {
        return None;
    }

    Some(words.join(" | "))
}

/// Prefix a comma-separated column list with a table alias, e.g.
/// `aliased_cols("id, title", "k")` -> `"k.id, k.title"`. Used by join
/// queries (RRF hydration, reply threading) that need `SELECT k.*` replaced
/// with an explicit alias-qualified list — keeps the column set defined once.
pub(crate) fn aliased_cols(cols: &str, alias: &str) -> String {
    cols.split(", ")
        .map(|c| format!("{alias}.{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub mod audit_log_repo;
pub mod briefing_repo;
pub mod client_repo;
pub mod embedding_repo;
pub mod handoff_repo;
pub mod knowledge_repo;
pub mod suggest_repo;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn or_tsquery_multi_word() {
        assert_eq!(
            build_or_tsquery_text("disk space running low"),
            Some("disk | space | running | low".to_string())
        );
    }

    #[test]
    fn or_tsquery_single_word_returns_none() {
        assert_eq!(build_or_tsquery_text("backup"), None);
    }

    #[test]
    fn or_tsquery_empty_returns_none() {
        assert_eq!(build_or_tsquery_text(""), None);
    }

    #[test]
    fn or_tsquery_strips_special_chars() {
        assert_eq!(
            build_or_tsquery_text("how do we handle (disk) space?"),
            Some("how | do | we | handle | disk | space".to_string())
        );
    }

    #[test]
    fn or_tsquery_preserves_hyphens() {
        assert_eq!(
            build_or_tsquery_text("cross-client safe"),
            Some("cross-client | safe".to_string())
        );
    }

    #[test]
    fn or_tsquery_bare_punctuation_filtered() {
        assert_eq!(build_or_tsquery_text("- --"), None);
    }

    #[test]
    fn aliased_cols_prefixes_every_column() {
        assert_eq!(
            aliased_cols("id, title, body", "h"),
            "h.id, h.title, h.body"
        );
    }

    #[test]
    fn aliased_cols_single_column() {
        assert_eq!(aliased_cols("id", "k"), "k.id");
    }
}
