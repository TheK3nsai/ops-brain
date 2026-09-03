//! Shared bounded-page handling for list and search surfaces.

pub(crate) const MAX_PAGE_LIMIT: i64 = 200;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PageRequest {
    pub limit: i64,
    pub limit_clamped: bool,
}

impl PageRequest {
    pub fn new(requested: Option<i64>, default: i64) -> Self {
        let requested = requested.unwrap_or(default);
        let limit = requested.clamp(1, MAX_PAGE_LIMIT);
        Self {
            limit,
            limit_clamped: requested != limit,
        }
    }

    /// Ask repositories for one row beyond the public page so callers can
    /// distinguish a complete page from a page with older results omitted.
    pub fn fetch_limit(self) -> i64 {
        self.limit + 1
    }

    /// Remove the probe row, returning whether it existed.
    pub fn trim<T>(self, items: &mut Vec<T>) -> bool {
        let has_more = items.len() > self.limit as usize;
        items.truncate(self.limit as usize);
        has_more
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_high_and_low_limit_clamps() {
        let high = PageRequest::new(Some(500), 20);
        assert_eq!(high.limit, 200);
        assert!(high.limit_clamped);
        assert_eq!(high.fetch_limit(), 201);

        let low = PageRequest::new(Some(0), 20);
        assert_eq!(low.limit, 1);
        assert!(low.limit_clamped);
        assert_eq!(low.fetch_limit(), 2);
    }

    #[test]
    fn probe_row_sets_has_more_and_is_not_returned() {
        let page = PageRequest::new(Some(2), 20);
        let mut items = vec![1, 2, 3];
        assert!(page.trim(&mut items));
        assert_eq!(items, vec![1, 2]);

        let mut complete = vec![1, 2];
        assert!(!page.trim(&mut complete));
        assert_eq!(complete, vec![1, 2]);
    }
}
