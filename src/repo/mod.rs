/// Build an OR-joined websearch query for FTS fallback.
/// Returns None if fewer than 2 searchable terms survive (OR is pointless with
/// 0-1 words). Used with `websearch_to_tsquery` when AND-based matching returns
/// zero results, so malformed punctuation can never raise a PostgreSQL tsquery
/// syntax error and separator-joined identifiers (`ops_brain`, `foo/bar`,
/// `v8.1.0`) split into their indexed tokens instead of concatenating.
pub(crate) fn build_or_tsquery_text(query: &str) -> Option<String> {
    let words: Vec<&str> = query
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .flat_map(|token| token.split("--"))
        .map(|token| token.trim_matches('-'))
        .filter(|w| !w.is_empty() && !w.eq_ignore_ascii_case("or"))
        .collect();

    if words.len() < 2 {
        return None;
    }

    Some(words.join(" OR "))
}

/// Handoff filter for a caller restricted to its own traffic: the row must
/// name `$param` as sender or recipient. The parameter is always bound (as
/// NULL when unrestricted), so placeholder numbering stays fixed. `prefix` is
/// a table alias with its dot (`"r."`), or empty.
pub(crate) fn party_clause(param: u32, prefix: &str) -> String {
    // A `<base>-*` family binding matches `<base>` and `<base>-<suffix>`
    // (mirrors `auth::slug_matches`). starts_with avoids LIKE, where `_` in a
    // slug would be a wildcard.
    let m = |col: String| {
        format!(
            "(LOWER({col}) = LOWER(${param}) OR (RIGHT(${param}, 2) = '-*' AND \
             (LOWER({col}) = LOWER(LEFT(${param}, -2)) OR (LENGTH({col}) > LENGTH(${param}) - 1 \
             AND STARTS_WITH(LOWER({col}), LOWER(LEFT(${param}, -1)))))))"
        )
    };
    format!(
        "(${param}::text IS NULL OR {} OR {})",
        m(format!("{prefix}from_agent")),
        m(format!("{prefix}to_agent"))
    )
}

pub mod handoff_repo;
pub mod reader_repo;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn or_tsquery_multi_word() {
        assert_eq!(
            build_or_tsquery_text("disk space running low"),
            Some("disk OR space OR running OR low".to_string())
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
            Some("how OR do OR we OR handle OR disk OR space".to_string())
        );
    }

    #[test]
    fn or_tsquery_preserves_hyphens() {
        assert_eq!(
            build_or_tsquery_text("cross-client safe"),
            Some("cross-client OR safe".to_string())
        );
    }

    #[test]
    fn or_tsquery_splits_separators_and_drops_bare_hyphens() {
        assert_eq!(build_or_tsquery_text("- --"), None);
        assert_eq!(build_or_tsquery_text("-- --"), None);
        assert_eq!(build_or_tsquery_text("(--) foo"), None);
        assert_eq!(
            build_or_tsquery_text("(--) foo bar"),
            Some("foo OR bar".to_string())
        );
        assert_eq!(
            build_or_tsquery_text("(-foo) bar-"),
            Some("foo OR bar".to_string())
        );
        assert_eq!(
            build_or_tsquery_text("ops_brain foo/bar v8.1.0"),
            Some("ops OR brain OR foo OR bar OR v8 OR 1 OR 0".to_string())
        );
        assert_eq!(
            build_or_tsquery_text("foo--bar baz"),
            Some("foo OR bar OR baz".to_string())
        );
    }
}
