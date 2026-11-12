//! The `--expect` baseline: case ids expected to pass, so CI fails only on regressions while
//! methods are still being built.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

use crate::error::Error;
use crate::judge::{CaseResult, Outcome};

const HEADER: &str = "\
# Conformance cases expected to pass (exactly or with ignores), one id per line.
# A listed case that fails is a regression. Update after a method improves with:
#   scripts/conformance.sh --write-expect conformance/expected.txt
# or: lfm-conformance run --base-url <url> --write-expect <this file>
";

/// The comparison of a run with the baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// The baseline file.
    pub file: String,
    /// Listed cases that failed in this run.
    pub regressions: Vec<String>,
    /// Passing cases the baseline does not list yet.
    pub new_passes: Vec<String>,
}

impl Check {
    /// Compares the run's results with the listed ids. Listed ids the run did not include
    /// count as neither.
    #[must_use]
    pub fn new(file: &Path, listed: &BTreeSet<String>, results: &[CaseResult]) -> Self {
        let passing = |r: &&CaseResult| r.outcome != Outcome::Failed;
        Self {
            file: file.display().to_string(),
            regressions: results
                .iter()
                .filter(|r| !passing(r) && listed.contains(&r.id))
                .map(|r| r.id.clone())
                .collect(),
            new_passes: results
                .iter()
                .filter(|r| passing(r) && !listed.contains(&r.id))
                .map(|r| r.id.clone())
                .collect(),
        }
    }

    pub(crate) fn summary(&self) -> String {
        let mut out = format!(
            "baseline {}: {} regressions, {} new passes\n",
            self.file,
            self.regressions.len(),
            self.new_passes.len()
        );
        for id in &self.regressions {
            let _ = writeln!(out, "  regression: {id}");
        }
        if !self.new_passes.is_empty() {
            let _ = writeln!(
                out,
                "  update the baseline with --write-expect {}",
                self.file
            );
        }
        out
    }
}

/// Reads a baseline. Blank lines and `#` comments are skipped.
pub fn read(path: &Path) -> Result<BTreeSet<String>, Error> {
    let text = std::fs::read_to_string(path).map_err(Error::io(path))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

/// Writes the passing ids of this run, keeping listed ids the run did not include.
pub fn write(path: &Path, results: &[CaseResult]) -> Result<(), Error> {
    let previous = if path.exists() {
        read(path)?
    } else {
        BTreeSet::new()
    };
    let run: BTreeSet<&str> = results.iter().map(|r| r.id.as_str()).collect();
    let ids: BTreeSet<&str> = previous
        .iter()
        .map(String::as_str)
        .filter(|id| !run.contains(id))
        .chain(
            results
                .iter()
                .filter(|r| r.outcome != Outcome::Failed)
                .map(|r| r.id.as_str()),
        )
        .collect();
    let body: String = ids.iter().flat_map(|id| [id, "\n"]).collect();
    std::fs::write(path, format!("{HEADER}{body}")).map_err(Error::io(path))
}
