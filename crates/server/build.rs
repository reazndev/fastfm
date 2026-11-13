//! Rebuilds the server when the web UI build changes, so the embedded files stay current.
//!
//! Cargo treats a missing path as always changed, so before the first UI build this watches
//! `web/` instead, where `dist/` will appear.

use std::path::Path;

fn main() {
    let watched = ["../../web/dist", "../../web"]
        .into_iter()
        .find(|path| Path::new(path).exists())
        .unwrap_or("build.rs");
    println!("cargo::rerun-if-changed={watched}");
}
