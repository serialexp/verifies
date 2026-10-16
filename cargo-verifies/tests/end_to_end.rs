//! Build and run `tests/fixture` under nextest, annotate its JUnit report,
//! and check every test case's keys and status. Needs `cargo-nextest`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use quick_xml::Reader;
use quick_xml::events::Event;

const FILTER: [&str; 4] = ["--profile", "ci", "-E", "not test(filtered_out)"];

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture")
}

/// A command in the fixture, isolated from the nextest run executing this
/// test: its own target directory, none of the parent's `NEXTEST_*`
/// settings, and plain output whatever colour setting the caller forces.
fn command(program: impl AsRef<std::ffi::OsStr>, target: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(fixture())
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_TERM_COLOR", "never");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NEXTEST") {
            command.env_remove(key);
        }
    }
    command
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// `(classname, name) → (requirements value, status)` for every test case.
fn cases(xml: &[u8]) -> BTreeMap<(String, String), (Option<String>, &'static str)> {
    let mut out = BTreeMap::new();
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut current: Option<(String, String)> = None;
    let attr = |e: &quick_xml::events::BytesStart<'_>, name: &str| {
        e.try_get_attribute(name)
            .unwrap()
            .map(|a| a.unescape_value().unwrap().into_owned())
    };
    loop {
        match reader.read_event_into(&mut buf).unwrap() {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                b"testcase" => {
                    let id = (attr(&e, "classname").unwrap(), attr(&e, "name").unwrap());
                    out.insert(id.clone(), (None, "passed"));
                    current = Some(id);
                }
                b"property" if attr(&e, "name").as_deref() == Some("requirements") => {
                    out.get_mut(current.as_ref().unwrap()).unwrap().0 = attr(&e, "value");
                }
                b"skipped" => out.get_mut(current.as_ref().unwrap()).unwrap().1 = "skipped",
                b"failure" => out.get_mut(current.as_ref().unwrap()).unwrap().1 = "failed",
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }
    out
}

#[test]
fn annotates_a_nextest_report() {
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("verifies-fixture");
    let cargo: OsString = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());

    let run = command(&cargo, &target)
        .args(["nextest", "run", "--no-fail-fast"])
        .args(FILTER)
        .output()
        .expect("running cargo nextest (is cargo-nextest installed?)");
    let run_text = text(&run);
    // `fails` fails on purpose; everything else passes.
    assert!(
        run_text.contains("12 tests run: 11 passed, 1 failed"),
        "{run_text}"
    );

    let report = fixture().join("target/nextest/ci/junit.xml");
    let annotated = target.join("annotated.xml");
    let verify = command(env!("CARGO_BIN_EXE_cargo-verifies"), &target)
        .env("CARGO", &cargo)
        .args(["verifies", "junit"])
        .arg(&report)
        .arg("--output")
        .arg(&annotated)
        .arg("--")
        .args(FILTER)
        .output()
        .unwrap();
    let verify_text = text(&verify);
    assert!(verify.status.success(), "{verify_text}");
    assert!(
        verify_text.contains("`integration::helper` in `verifies-fixture::integration`"),
        "a #[verifies] helper that is not a test is reported: {verify_text}"
    );
    assert!(
        verify_text.contains("11 cases annotated, 2 added as skipped"),
        "{verify_text}"
    );

    let unit = "verifies-fixture";
    let integration = "verifies-fixture::integration";
    let expected: BTreeMap<(String, String), (Option<String>, &str)> = [
        (unit, "tests::doubles", Some("REQ-1.1"), "passed"),
        (
            unit,
            "tests::stacked",
            Some("REQ-1.1,REQ-1.2,REQ-2"),
            "passed",
        ),
        (unit, "tests::ignored", Some("REQ-3"), "skipped"),
        (unit, "tests::nested::deep", Some("REQ-4"), "passed"),
        (unit, "tests::untraced", None, "passed"),
        (integration, "tokio_below", Some("REQ-5"), "passed"),
        (integration, "tokio_above", Some("REQ-6"), "passed"),
        (integration, "rstest_cases::case_1", Some("REQ-7"), "passed"),
        (integration, "rstest_cases::case_2", Some("REQ-7"), "passed"),
        (
            integration,
            "test_cases::test_case_1_one",
            Some("REQ-8"),
            "passed",
        ),
        (
            integration,
            "test_cases::test_case_2_two",
            Some("REQ-8"),
            "passed",
        ),
        (
            integration,
            "test_cases_below::test_case_1_one",
            Some("REQ-12"),
            "passed",
        ),
        (integration, "filtered_out", Some("REQ-9"), "skipped"),
        (integration, "fails", Some("REQ-10"), "failed"),
    ]
    .into_iter()
    .map(|(binary, name, keys, status)| {
        (
            (binary.to_string(), name.to_string()),
            (keys.map(str::to_string), status),
        )
    })
    .collect();
    let output = std::fs::read(&annotated).unwrap();
    assert_eq!(cases(&output), expected);

    // In place, and a second time over its own output: nothing changes.
    std::fs::copy(&annotated, target.join("again.xml")).unwrap();
    let again = command(env!("CARGO_BIN_EXE_cargo-verifies"), &target)
        .env("CARGO", &cargo)
        .args(["verifies", "junit"])
        .arg(target.join("again.xml"))
        .arg("--")
        .args(FILTER)
        .output()
        .unwrap();
    assert!(again.status.success(), "{}", text(&again));
    assert!(
        text(&again).contains("0 cases annotated, 0 added as skipped"),
        "{}",
        text(&again)
    );
    assert_eq!(std::fs::read(target.join("again.xml")).unwrap(), output);
}
