//! `cargo verifies junit`: copy the requirement keys that `#[verifies(...)]`
//! left in Rust test binaries into nextest's JUnit report.
//!
//! After `cargo nextest run`, this
//!
//! 1. asks nextest for the test binaries and their tests
//!    (`cargo nextest list --message-format json`);
//! 2. reads every binary's records (`crates/verifies/FORMAT.md`) and links
//!    each to the tests it names ([`link`]);
//! 3. rewrites the JUnit report: each linked test case gains a
//!    `requirements` property, and a linked test missing from the report
//!    (ignored or filtered out) is added as skipped so its link is kept
//!    ([`junit`]).
//!
//! A malformed or newer-version record stops it before anything is written.

pub mod junit;
pub mod key;
pub mod link;
pub mod nextest;
pub mod record;

use std::path::Path;

pub use link::{Links, TestId, Warning};

/// What [`annotate`] did.
#[derive(Debug)]
pub struct Outcome {
    pub links: Links,
    pub warnings: Vec<Warning>,
    pub rewritten: junit::Rewritten,
}

/// Link the records in `list`'s binaries and rewrite `report` with them.
/// Writes nothing; the caller decides where the result goes.
pub fn annotate(list: &nextest::TestList, report: &[u8]) -> anyhow::Result<Outcome> {
    let (links, warnings) = link::link(list, |path: &Path| {
        // A binary nextest listed but that is gone has no records to give;
        // a binary it skipped may never have been built.
        match record::read_binary(path) {
            Err(record::ReadError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(Vec::new())
            }
            result => result,
        }
    })?;
    let rewritten = junit::rewrite(report, &links)?;
    Ok(Outcome {
        links,
        warnings,
        rewritten,
    })
}
