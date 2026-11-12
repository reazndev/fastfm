//! `lfm-conformance run`: replays the fixtures against a server and scores the answers.
//! Exits 0 when nothing fails beyond the ignore rules (or, with `--expect`, nothing regresses),
//! 1 when something does, 2 when the run could not happen.

mod args;

use std::path::Path;
use std::process::ExitCode;

use lfm_conformance::client::Target;
use lfm_conformance::error::Error;
use lfm_conformance::ignore::Rules;
use lfm_conformance::report::Report;
use lfm_conformance::{baseline, fixture, run};

use crate::args::Args;

fn main() -> ExitCode {
    match conformance(std::env::args().skip(1)) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(err) => {
            console::error(&err.to_string());
            ExitCode::from(2)
        }
    }
}

fn conformance(args: impl Iterator<Item = String>) -> Result<bool, Error> {
    let args = Args::parse(args)?;
    let rules = args
        .ignore
        .as_deref()
        .map_or_else(|| Ok(Rules::default()), Rules::load)?;
    let cases = fixture::select(fixture::load(&args.fixtures)?, &args.only, args.format);
    if cases.is_empty() {
        return Err(Error::Usage(
            "no fixture matches --only/--format".to_owned(),
        ));
    }
    let target = Target::new(&args.base_url, &args.api_key)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(Error::io("tokio runtime"))?;
    let results = runtime.block_on(run::run(&target, &cases, &rules, args.pace));
    let mut report = Report::build(target.base_url(), &cases, results, &rules);
    if let Some(path) = &args.expect {
        let listed = baseline::read(path)?;
        report.baseline = Some(baseline::Check::new(path, &listed, &report.cases));
    }
    if let Some(path) = &args.report {
        write_report(path, &report)?;
    }
    if let Some(path) = &args.write_expect {
        baseline::write(path, &report.cases)?;
    }
    console::summary(&report.summary());
    Ok(report.passed())
}

fn write_report(path: &Path, report: &Report) -> Result<(), Error> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(Error::io(dir))?;
    }
    let text = serde_json::to_string_pretty(report)?;
    std::fs::write(path, text + "\n").map_err(Error::io(path))
}

#[allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool whose output is the summary; no tracing subscriber here"
)]
mod console {
    pub(crate) fn summary(text: &str) {
        print!("{text}");
    }

    pub(crate) fn error(text: &str) {
        eprintln!("lfm-conformance: {text}");
    }
}
