//! `page` and `limit` parsing with per-method defaults and maximums.

use crate::params::Params;

/// A method's paging rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSpec {
    /// `limit` when the request gives none.
    pub default_limit: u32,
    /// The largest `limit` the method honours; larger values are clamped.
    pub max_limit: u32,
}

impl PageSpec {
    /// Rules with the given default and maximum `limit`.
    #[must_use]
    pub const fn new(default_limit: u32, max_limit: u32) -> Self {
        Self {
            default_limit,
            max_limit,
        }
    }
}

/// The page a request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paging {
    /// The 1-based page number.
    pub page: u32,
    /// Items per page.
    pub limit: u32,
}

impl Paging {
    /// Reads `page` and `limit`. Missing, zero or unparsable values fall back to page 1 and the
    /// default limit.
    #[must_use]
    pub fn parse(params: &Params, spec: PageSpec) -> Self {
        let positive = |name| {
            params
                .number(name)
                .filter(|n| *n > 0)
                .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        };
        Self {
            page: positive("page").unwrap_or(1),
            limit: positive("limit")
                .unwrap_or(spec.default_limit)
                .min(spec.max_limit),
        }
    }

    /// The number of items before this page.
    #[must_use]
    pub fn offset(self) -> u64 {
        u64::from(self.page.saturating_sub(1)) * u64::from(self.limit)
    }

    /// `totalPages` for a total item count: zero when there are no items.
    #[must_use]
    pub fn total_pages(self, total: u64) -> u64 {
        total.div_ceil(u64::from(self.limit.max(1)))
    }
}
