//! `period`, `from` and `to`.

use crate::params::Params;

const DAY: u64 = 86_400;

/// A chart period as accepted by `period=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Period {
    /// All time.
    Overall,
    /// The last 7 days.
    Week,
    /// The last month.
    Month,
    /// The last 3 months.
    Quarter,
    /// The last 6 months.
    HalfYear,
    /// The last 12 months.
    Year,
}

impl Period {
    /// Reads `period`. Missing or unknown values mean overall.
    #[must_use]
    pub fn parse(params: &Params) -> Self {
        match params.optional("period") {
            Some("7day") => Self::Week,
            Some("1month") => Self::Month,
            Some("3month") => Self::Quarter,
            Some("6month") => Self::HalfYear,
            Some("12month") => Self::Year,
            _ => Self::Overall,
        }
    }

    /// The value as it appears in `period=` and in `@attr`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Overall => "overall",
            Self::Week => "7day",
            Self::Month => "1month",
            Self::Quarter => "3month",
            Self::HalfYear => "6month",
            Self::Year => "12month",
        }
    }

    /// The time range ending at `now` (Unix seconds). Months count as 30 days.
    #[must_use]
    pub fn range(self, now: u64) -> TimeRange {
        let days = match self {
            Self::Overall => return TimeRange::default(),
            Self::Week => 7,
            Self::Month => 30,
            Self::Quarter => 90,
            Self::HalfYear => 180,
            Self::Year => 365,
        };
        TimeRange {
            from: Some(now.saturating_sub(days * DAY)),
            to: Some(now),
        }
    }
}

/// A time range in Unix seconds; `None` leaves that side open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TimeRange {
    /// Inclusive start.
    pub from: Option<u64>,
    /// End.
    pub to: Option<u64>,
}

impl TimeRange {
    /// Reads `from` and `to`. Unparsable values are ignored.
    #[must_use]
    pub fn parse(params: &Params) -> Self {
        Self {
            from: params.number("from"),
            to: params.number("to"),
        }
    }

    /// Whether `uts` falls inside the range, treating `to` as inclusive like Last.fm.
    #[must_use]
    pub fn contains(self, uts: u64) -> bool {
        self.from.is_none_or(|from| uts >= from) && self.to.is_none_or(|to| uts <= to)
    }
}
