//! Matching records to the tests nextest knows, by binary and test name.

use std::collections::{BTreeMap, BTreeSet};

use crate::nextest::TestList;
use crate::record::{ReadError, Record};

/// A test as nextest's JUnit report identifies it: `classname` is the binary
/// id and `name` the test name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TestId {
    pub binary_id: String,
    pub name: String,
}

/// The keys of every linked test.
pub type Links = BTreeMap<TestId, BTreeSet<String>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// A record whose path is not a test in the binary it was found in: the
    /// attribute is on a function that is not a test, or the record came in
    /// with a dependency.
    NotATest { binary_id: String, path: String },
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotATest { binary_id, path } => write!(
                f,
                "`{path}` in `{binary_id}` has #[verifies] but is not one of its tests; ignored"
            ),
        }
    }
}

/// Link every record in the listed binaries to its tests. `read` reads one
/// binary's records.
///
/// A record names a test function. It links the test of that name, or, when
/// there is none, every test generated under it (`name::case_1`, as rstest
/// and test-case generate). A binary nextest skipped has no test list, so its
/// records are linked by their own names.
pub fn link(
    list: &TestList,
    mut read: impl FnMut(&std::path::Path) -> Result<Vec<Record>, ReadError>,
) -> Result<(Links, Vec<Warning>), ReadError> {
    let mut links = Links::new();
    let mut warnings = Vec::new();
    for suite in list.rust_suites.values() {
        for record in read(&suite.binary_path)? {
            let mut add = |name: &str| {
                links
                    .entry(TestId {
                        binary_id: suite.binary_id.clone(),
                        name: name.to_string(),
                    })
                    .or_default()
                    .extend(record.keys.iter().cloned());
            };
            let Some(name) = record.test_name() else {
                warnings.push(Warning::NotATest {
                    binary_id: suite.binary_id.clone(),
                    path: record.path.clone(),
                });
                continue;
            };
            if !suite.is_listed() {
                add(name);
                continue;
            }
            if suite.testcases.contains_key(name) {
                add(name);
                continue;
            }
            let prefix = format!("{name}::");
            let generated: Vec<&String> = suite
                .testcases
                .range::<String, _>(prefix.clone()..)
                .take_while(|(test, _)| test.starts_with(&prefix))
                .map(|(test, _)| test)
                .collect();
            if generated.is_empty() {
                warnings.push(Warning::NotATest {
                    binary_id: suite.binary_id.clone(),
                    path: record.path.clone(),
                });
            }
            for test in generated {
                add(test);
            }
        }
    }
    Ok((links, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn list() -> TestList {
        TestList::parse(
            br#"{"rust-suites": {
                "krate": {"binary-id": "krate", "binary-path": "/krate",
                          "testcases": {"tests::plain": {}, "tests::cases::one": {},
                                        "tests::cases::two": {}, "tests::casesx": {}}},
                "krate::it": {"binary-id": "krate::it", "binary-path": "/it",
                              "status": "skipped", "testcases": {}}
            }}"#,
        )
        .unwrap()
    }

    fn record(path: &str, keys: &[&str]) -> Record {
        Record {
            path: path.into(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
        }
    }

    fn id(binary_id: &str, name: &str) -> TestId {
        TestId {
            binary_id: binary_id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn records_link_to_tests_and_generated_cases() {
        let (links, warnings) = link(&list(), |path| {
            Ok(if path == Path::new("/krate") {
                vec![
                    record("krate::tests::plain", &["REQ-1"]),
                    record("krate::tests::plain", &["REQ-2", "REQ-1"]),
                    record("krate::tests::cases", &["REQ-3"]),
                    record("krate::helpers::setup", &["REQ-4"]),
                ]
            } else {
                vec![record("it::not_run", &["REQ-5"])]
            })
        })
        .unwrap();
        let keys = |set: &[&str]| set.iter().map(|k| k.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(
            links,
            Links::from([
                (id("krate", "tests::plain"), keys(&["REQ-1", "REQ-2"])),
                (id("krate", "tests::cases::one"), keys(&["REQ-3"])),
                (id("krate", "tests::cases::two"), keys(&["REQ-3"])),
                (id("krate::it", "not_run"), keys(&["REQ-5"])),
            ])
        );
        assert_eq!(
            warnings,
            [Warning::NotATest {
                binary_id: "krate".into(),
                path: "krate::helpers::setup".into(),
            }]
        );
    }
}
