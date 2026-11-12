//! Date nodes: `<date uts="1793990654">06 Nov 2026, 18:44</date>`, always in UTC.

use std::fmt::Write as _;

use crate::tree::{Node, Str};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A date element with a `uts` attribute and Last.fm's display text.
#[must_use]
pub fn date_node(name: impl Into<Str>, uts: u64) -> Node {
    Node::leaf(name, date_text(uts)).attr("uts", uts)
}

/// Last.fm's display format, `DD Mon YYYY, HH:MM` in UTC with a zero-padded day.
#[must_use]
pub fn date_text(uts: u64) -> String {
    let days = uts / 86_400;
    let seconds = uts % 86_400;
    let (year, month, day) = civil_from_days(days);
    let month_name = MONTHS
        .get(usize::try_from(month.saturating_sub(1)).unwrap_or(0))
        .copied()
        .unwrap_or("Jan");
    let mut out = String::with_capacity(18);
    let _ = write!(
        out,
        "{day:02} {month_name} {year}, {:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60
    );
    out
}

/// Converts days since 1970-01-01 to (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}
