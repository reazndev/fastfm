//! Ignore rules: differences that legitimately vary, each with a reason.
//!
//! ```toml
//! [[rule]]
//! method = "artist.getInfo"          # glob; `*` stops at `.`, `**` does not
//! variant = "*"                      # optional glob, default every variant
//! format = "json"                    # optional, default both
//! path = "artist.stats.listeners"    # glob over difference paths
//! categories = ["value"]             # optional, default ["value"]
//! values = ["a", "b"]                # optional: only when both sides are one of these
//! reason = "Global listener count, changes by the minute."
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::diff::{Category, Difference};
use crate::error::Error;
use crate::fixture::{Case, Format};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    rule: Vec<RuleSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleSpec {
    method: String,
    variant: Option<String>,
    format: Option<Format>,
    path: String,
    categories: Option<Vec<String>>,
    #[serde(default)]
    values: Vec<String>,
    reason: String,
}

/// One rule, numbered from 1 in file order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rule {
    /// Position in the file, from 1.
    pub id: usize,
    /// Method glob.
    pub method: String,
    /// Variant glob.
    pub variant: String,
    /// Format, or both.
    pub format: Option<Format>,
    /// Path glob.
    pub path: String,
    /// Categories the rule covers.
    pub categories: Vec<Category>,
    /// When not empty, the rule covers a difference only if the recorded and the received
    /// value (as shown in the report) are both in this list.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// Why the difference is legitimate.
    pub reason: String,
}

impl Rule {
    /// Whether the rule is about this case at all, regardless of path.
    #[must_use]
    pub fn selects(&self, case: &Case) -> bool {
        glob(&self.method, &case.method)
            && glob(&self.variant, &case.variant)
            && self.format.is_none_or(|f| f == case.format)
    }

    /// Whether the rule covers this difference in this case.
    #[must_use]
    pub fn covers(&self, case: &Case, difference: &Difference) -> bool {
        self.selects(case)
            && self.categories.contains(&difference.category)
            && glob(&self.path, &difference.path)
            && (self.values.is_empty()
                || (self.values.contains(&difference.expected)
                    && self.values.contains(&difference.actual)))
    }
}

/// The loaded rule set.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    rules: Vec<Rule>,
}

impl Rules {
    /// Reads and validates an ignore file.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path).map_err(Error::io(path))?;
        Self::parse(&text).map_err(|message| Error::Ignore {
            path: path.to_owned(),
            message,
        })
    }

    /// Parses ignore-file text. Every rule needs a non-empty reason, method and path.
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let rules = file
            .rule
            .into_iter()
            .enumerate()
            .map(|(i, spec)| build(i + 1, spec))
            .collect::<Result<_, _>>()?;
        Ok(Self { rules })
    }

    /// Every rule in file order.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// The first rule that covers the difference.
    #[must_use]
    pub fn matching(&self, case: &Case, difference: &Difference) -> Option<&Rule> {
        self.rules.iter().find(|r| r.covers(case, difference))
    }
}

fn build(id: usize, spec: RuleSpec) -> Result<Rule, String> {
    let problem = |what: &str| format!("rule {id}: {what}");
    if spec.reason.trim().is_empty() {
        return Err(problem("needs a reason"));
    }
    if spec.method.trim().is_empty() || spec.path.trim().is_empty() {
        return Err(problem("needs a method and a path"));
    }
    let categories = spec
        .categories
        .unwrap_or_else(|| vec![Category::Value.as_str().to_owned()])
        .iter()
        .map(|name| {
            Category::parse(name).ok_or_else(|| problem(&format!("unknown category {name}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if categories.is_empty() {
        return Err(problem("categories is empty"));
    }
    Ok(Rule {
        id,
        method: spec.method,
        variant: spec.variant.unwrap_or_else(|| "*".to_owned()),
        format: spec.format,
        path: spec.path,
        categories,
        values: spec.values,
        reason: spec.reason.trim().to_owned(),
    })
}

/// Glob match: `**` matches anything, `*` anything up to the next `.`, other characters
/// match themselves.
#[must_use]
pub fn glob(pattern: &str, text: &str) -> bool {
    glob_bytes(pattern.as_bytes(), text.as_bytes())
}

fn glob_bytes(pattern: &[u8], text: &[u8]) -> bool {
    match pattern {
        [] => text.is_empty(),
        [b'*', b'*', rest @ ..] => {
            (0..=text.len()).any(|i| glob_bytes(rest, text.get(i..).unwrap_or_default()))
        }
        [b'*', rest @ ..] => {
            let segment = text.iter().position(|&b| b == b'.').unwrap_or(text.len());
            (0..=segment).any(|i| glob_bytes(rest, text.get(i..).unwrap_or_default()))
        }
        [c, rest @ ..] => {
            text.first() == Some(c) && glob_bytes(rest, text.get(1..).unwrap_or_default())
        }
    }
}
