//! The parts of `cargo nextest list --message-format json` this tool reads.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TestList {
    pub rust_suites: BTreeMap<String, Suite>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Suite {
    pub binary_id: String,
    pub binary_path: PathBuf,
    /// `listed`, or why nextest did not list the binary (it matched no
    /// filter), in which case `testcases` is empty. Older nextest versions
    /// omit it and always list.
    #[serde(default = "listed")]
    pub status: String,
    /// Every test in the binary, run or not; the values are not needed.
    #[serde(default)]
    pub testcases: BTreeMap<String, serde::de::IgnoredAny>,
}

impl Suite {
    pub fn is_listed(&self) -> bool {
        self.status == "listed"
    }
}

fn listed() -> String {
    "listed".to_string()
}

impl TestList {
    pub fn parse(json: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(json)
    }

    /// Run `cargo nextest list` with `args` (the same package, feature and
    /// profile selection the test run used). Building is nextest's: it
    /// rebuilds nothing that the run already built.
    pub fn from_nextest(cargo: &OsString, args: &[OsString]) -> anyhow::Result<Self> {
        let output = Command::new(cargo)
            .args(["nextest", "list", "--message-format", "json"])
            .args(args)
            .stderr(Stdio::inherit())
            .output()
            .map_err(|error| anyhow::anyhow!("running `cargo nextest list`: {error}"))?;
        if !output.status.success() {
            anyhow::bail!("`cargo nextest list` failed ({})", output.status);
        }
        Self::parse(&output.stdout)
            .map_err(|error| anyhow::anyhow!("reading `cargo nextest list` output: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nextest_list_output() {
        let json = br#"{
            "rust-build-meta": {},
            "test-count": 2,
            "rust-suites": {
                "spike": {
                    "package-name": "spike", "binary-id": "spike", "binary-name": "spike",
                    "kind": "lib", "binary-path": "/t/spike-1", "build-platform": "target",
                    "cwd": "/t", "status": "listed",
                    "testcases": {"tests::adds": {"kind": "test", "ignored": false,
                                  "filter-match": {"status": "matches"}}}
                },
                "spike::it": {
                    "package-name": "spike", "binary-id": "spike::it", "binary-name": "it",
                    "kind": "test", "binary-path": "/t/it-1", "build-platform": "target",
                    "cwd": "/t", "status": "skipped", "testcases": {}
                },
                "old": {"binary-id": "old", "binary-path": "/t/old-1", "testcases": {}}
            }
        }"#;
        let list = TestList::parse(json).unwrap();
        let spike = &list.rust_suites["spike"];
        assert!(spike.is_listed());
        assert!(spike.testcases.contains_key("tests::adds"));
        assert!(!list.rust_suites["spike::it"].is_listed());
        assert!(
            list.rust_suites["old"].is_listed(),
            "no status means listed"
        );
    }
}
