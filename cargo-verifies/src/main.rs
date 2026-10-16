use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};

use cargo_verifies::nextest::TestList;

/// Invoked by cargo as `cargo-verifies verifies <command>`.
#[derive(Debug, Parser)]
#[command(name = "cargo", bin_name = "cargo")]
enum Cargo {
    /// Requirement traces for Rust tests marked with #[verifies(...)]
    #[command(version)]
    Verifies(VerifiesArgs),
}

#[derive(Debug, Args)]
struct VerifiesArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Add each test's requirement keys to nextest's JUnit report
    ///
    /// Run after `cargo nextest run`. Every linked test case gains a
    /// `requirements` property; a linked test that is not in the report
    /// (ignored or filtered out) is added as skipped.
    Junit(JunitArgs),
}

#[derive(Debug, Args)]
struct JunitArgs {
    /// The JUnit report nextest wrote; rewritten in place unless --output is
    /// given
    report: PathBuf,

    /// Write the rewritten report here instead
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Read the test list from this file (the output of
    /// `cargo nextest list --message-format json`) instead of running nextest
    #[arg(long, value_name = "PATH")]
    list_json: Option<PathBuf>,

    /// Arguments for `cargo nextest list`: the package, feature, target and
    /// profile selection the test run used
    #[arg(last = true, value_name = "NEXTEST_ARGS")]
    nextest_args: Vec<OsString>,
}

fn main() -> ExitCode {
    let Cargo::Verifies(args) = Cargo::parse();
    match args.command {
        Command::Junit(args) => match junit(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error:#}");
                ExitCode::FAILURE
            }
        },
    }
}

fn junit(args: JunitArgs) -> anyhow::Result<()> {
    let list = match &args.list_json {
        Some(path) => {
            let json =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            TestList::parse(&json).with_context(|| format!("reading {}", path.display()))?
        }
        None => {
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            TestList::from_nextest(&cargo, &args.nextest_args)?
        }
    };
    let report = std::fs::read(&args.report)
        .with_context(|| format!("reading {}", args.report.display()))?;
    let outcome = cargo_verifies::annotate(&list, &report)?;
    for warning in &outcome.warnings {
        eprintln!("warning: {warning}");
    }
    let output = args.output.as_deref().unwrap_or(&args.report);
    write_atomically(output, &outcome.rewritten.xml)?;
    let keys: std::collections::BTreeSet<&String> = outcome.links.values().flatten().collect();
    eprintln!(
        "cargo-verifies: {} tests trace {} requirement keys; {} cases annotated, {} added as skipped",
        outcome.links.len(),
        keys.len(),
        outcome.rewritten.annotated,
        outcome.rewritten.added,
    );
    Ok(())
}

/// Replace `path` only once the whole report is written, so a failure never
/// leaves a truncated report for the upload step to send.
fn write_atomically(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("creating a file in {}", dir.display()))?;
    temp.write_all(bytes)
        .and_then(|()| temp.as_file().sync_all())
        .with_context(|| format!("writing {}", temp.path().display()))?;
    temp.persist(path)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_as_a_cargo_subcommand() {
        let Cargo::Verifies(args) = Cargo::try_parse_from([
            "cargo-verifies",
            "verifies",
            "junit",
            "target/nextest/ci/junit.xml",
            "--",
            "--profile",
            "ci",
            "--workspace",
        ])
        .unwrap();
        let Command::Junit(junit) = args.command;
        assert_eq!(junit.report, Path::new("target/nextest/ci/junit.xml"));
        assert_eq!(junit.nextest_args, ["--profile", "ci", "--workspace"]);
        assert!(Cargo::try_parse_from(["cargo-verifies", "verifies", "junit"]).is_err());
    }
}
