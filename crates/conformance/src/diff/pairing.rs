//! Pairs named children (JSON members, XML elements and attributes) by name and occurrence,
//! so repeated names pair up in order and the rest is missing, extra, or out of order.

use std::collections::BTreeMap;

use super::{Category, Difference};

/// A name and how many earlier siblings share it.
pub(super) type Key<'a> = (&'a str, usize);

/// The result of pairing two sibling lists.
#[derive(Debug)]
pub(super) struct Paired<'a, T> {
    /// In the fixture only, in fixture order.
    pub(super) missing: Vec<(Key<'a>, &'a T)>,
    /// In the answer only, in answer order.
    pub(super) extra: Vec<(Key<'a>, &'a T)>,
    /// On both sides, in fixture order.
    pub(super) both: Vec<(Key<'a>, &'a T, &'a T)>,
    want_order: Vec<&'a str>,
    got_order: Vec<&'a str>,
}

impl<T> Paired<'_, T> {
    /// One `order` difference when the shared names come in another order. Names are shown
    /// with `prefix` (`@` for attributes).
    pub(super) fn order_difference(&self, path: &str, prefix: &str) -> Option<Difference> {
        (self.want_order != self.got_order).then(|| {
            let show = |names: &[&str]| compact_names(names.iter().map(|n| format!("{prefix}{n}")));
            Difference::new(
                Category::Order,
                path,
                show(&self.want_order),
                show(&self.got_order),
            )
        })
    }
}

/// Pairs the two sides.
pub(super) fn pair<'a, T>(
    expected: impl IntoIterator<Item = (&'a str, &'a T)>,
    actual: impl IntoIterator<Item = (&'a str, &'a T)>,
) -> Paired<'a, T> {
    let want = keyed(expected);
    let got = keyed(actual);
    let only_in = |side: &[(Key<'a>, &'a T)], other: &[(Key<'a>, &'a T)]| {
        side.iter()
            .filter(|(k, _)| find(other, *k).is_none())
            .copied()
            .collect::<Vec<_>>()
    };
    let both: Vec<_> = want
        .iter()
        .filter_map(|(k, w)| find(&got, *k).map(|g| (*k, *w, g)))
        .collect();
    let got_order = got
        .iter()
        .filter(|(k, _)| find(&want, *k).is_some())
        .map(|((name, _), _)| *name)
        .collect();
    Paired {
        missing: only_in(&want, &got),
        extra: only_in(&got, &want),
        want_order: both.iter().map(|((name, _), _, _)| *name).collect(),
        got_order,
        both,
    }
}

fn keyed<'a, T>(items: impl IntoIterator<Item = (&'a str, &'a T)>) -> Vec<(Key<'a>, &'a T)> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    items
        .into_iter()
        .map(|(name, item)| {
            let count = seen.entry(name).or_insert(0);
            let key = (name, *count);
            *count += 1;
            (key, item)
        })
        .collect()
}

fn find<'a, T>(side: &[(Key<'a>, &'a T)], key: Key<'_>) -> Option<&'a T> {
    side.iter().find(|(k, _)| *k == key).map(|(_, item)| *item)
}

/// Lists names compactly, collapsing runs: `name, mbid, image×4`.
pub(super) fn compact_names(names: impl IntoIterator<Item = String>) -> String {
    let mut runs: Vec<(String, usize)> = Vec::new();
    for name in names {
        match runs.last_mut() {
            Some((last, count)) if *last == name => *count += 1,
            _ => runs.push((name, 1)),
        }
    }
    runs.iter()
        .map(|(name, count)| match count {
            1 => name.clone(),
            n => format!("{name}×{n}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
