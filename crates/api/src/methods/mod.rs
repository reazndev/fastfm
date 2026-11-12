//! Last.fm methods, one file per method under `methods/<package>/<method>.rs`.
//!
//! To add a method: create `methods/<package>/<method>.rs` with a unit struct that implements
//! [`Method`](crate::Method), declare it in `methods/<package>/mod.rs`, and add it to
//! [`registry`]. See [`example::Echo`] for the shape.

pub mod example;

use crate::registry::Registry;

/// Every Last.fm method the mirror serves.
#[must_use]
pub fn registry() -> Registry {
    Registry::new()
}
