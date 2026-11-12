//! Conformance harness: replays the recorded Last.fm fixtures against a server and reports
//! every difference by level (status, headers, body bytes, structure) with a precise path.

pub mod baseline;
pub mod client;
pub mod diff;
pub mod error;
pub mod fixture;
pub mod ignore;
pub mod judge;
pub mod report;
pub mod run;

pub use error::Error;
