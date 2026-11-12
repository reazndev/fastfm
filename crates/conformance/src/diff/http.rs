//! Status line and headers.

use std::collections::BTreeSet;

use super::{Category, Difference};
use crate::client::Received;
use crate::fixture::Recorded;

const ABSENT: &str = "(absent)";

/// Status, then content type, then every `access-control-*` header on either side.
pub(super) fn compare(expected: &Recorded, received: &Received) -> Vec<Difference> {
    let status = (expected.status != received.status).then(|| {
        Difference::new(
            Category::Status,
            "status",
            expected.status.to_string(),
            received.status.to_string(),
        )
    });
    let content_type = header_difference(Category::ContentType, "content-type", expected, received);
    let cors: BTreeSet<&str> = expected
        .headers
        .keys()
        .chain(received.headers.keys())
        .map(String::as_str)
        .filter(|name| name.starts_with("access-control-"))
        .collect();
    let headers = cors
        .into_iter()
        .filter_map(|name| header_difference(Category::Header, name, expected, received));
    status
        .into_iter()
        .chain(content_type)
        .chain(headers)
        .collect()
}

fn header_difference(
    category: Category,
    name: &str,
    expected: &Recorded,
    received: &Received,
) -> Option<Difference> {
    let want = expected.headers.get(name).map(String::as_str);
    let got = received.headers.get(name).map(String::as_str);
    (want != got).then(|| {
        Difference::new(
            category,
            &format!("header.{name}"),
            want.unwrap_or(ABSENT),
            got.unwrap_or(ABSENT),
        )
    })
}
