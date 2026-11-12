//! Command-line parsing for `lfm-conformance run`.

use std::path::PathBuf;

use lfm_conformance::Error;
use lfm_conformance::fixture::Format;
use lfm_conformance::run::Pace;

const KEY_VARIABLE: &str = "LFM_CONFORMANCE_API_KEY";
const DEFAULT_IGNORE: &str = "conformance/ignore.toml";

/// Everything `run` needs.
#[derive(Debug)]
pub(crate) struct Args {
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) only: Vec<String>,
    pub(crate) format: Option<Format>,
    pub(crate) report: Option<PathBuf>,
    pub(crate) ignore: Option<PathBuf>,
    pub(crate) fixtures: PathBuf,
    pub(crate) expect: Option<PathBuf>,
    pub(crate) write_expect: Option<PathBuf>,
    pub(crate) pace: Pace,
}

impl Args {
    /// Parses the arguments after the program name.
    pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, Error> {
        let mut args = args.into_iter();
        if args.next().as_deref() != Some("run") {
            return Err(usage("expected the `run` command"));
        }
        let mut parsed = Self {
            base_url: String::new(),
            api_key: std::env::var(KEY_VARIABLE).unwrap_or_default(),
            only: Vec::new(),
            format: None,
            report: None,
            ignore: None,
            fixtures: PathBuf::from("fixtures"),
            expect: None,
            write_expect: None,
            pace: Pace::default(),
        };
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| usage(&format!("{flag} needs a value")))?;
            parsed.set(&flag, value)?;
        }
        if parsed.base_url.is_empty() {
            return Err(usage("--base-url is required"));
        }
        if parsed.api_key.is_empty() {
            return Err(usage(&format!("--api-key or {KEY_VARIABLE} is required")));
        }
        if parsed.ignore.is_none() && PathBuf::from(DEFAULT_IGNORE).exists() {
            parsed.ignore = Some(PathBuf::from(DEFAULT_IGNORE));
        }
        Ok(parsed)
    }

    fn set(&mut self, flag: &str, value: String) -> Result<(), Error> {
        match flag {
            "--base-url" => self.base_url = value,
            "--api-key" => self.api_key = value,
            "--only" => self.only.push(value),
            "--format" => {
                self.format =
                    Some(Format::parse(&value).ok_or_else(|| usage("--format is xml or json"))?);
            }
            "--report" => self.report = Some(value.into()),
            "--ignore" => self.ignore = Some(value.into()),
            "--fixtures" => self.fixtures = value.into(),
            "--expect" => self.expect = Some(value.into()),
            "--write-expect" => self.write_expect = Some(value.into()),
            "--concurrency" => {
                self.pace.concurrency = value
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| usage("--concurrency is a positive number"))?;
            }
            "--rps" => {
                let rps: f64 = value
                    .parse()
                    .ok()
                    .filter(|n: &f64| *n > 0.0)
                    .ok_or_else(|| usage("--rps is a positive number"))?;
                self.pace.requests_per_second = Some(rps);
            }
            other => return Err(usage(&format!("unknown option {other}"))),
        }
        Ok(())
    }
}

fn usage(message: &str) -> Error {
    Error::Usage(message.to_owned())
}
