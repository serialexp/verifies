//! Rewriting nextest's JUnit report: a `requirements` property on every
//! linked test case, and a `<skipped/>` case for every linked test the report
//! does not contain, so the link survives an ignored or filtered-out test.
//!
//! Two streaming passes over the report: the first finds which linked tests
//! are present (and which already carry the property, so a second run changes
//! nothing), the second copies every event, adding what is missing and
//! adjusting the `tests`/`skipped` counts to match.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use crate::link::{Links, TestId};

/// The property the keys are written to: the Xray convention, which
/// Playwright's JUnit reporter and Gothab also use.
pub const PROPERTY: &str = "requirements";

const SKIPPED_MESSAGE: &str = "not in this test run (ignored or filtered out); cargo-verifies added it to keep its requirement links";

#[derive(Debug, thiserror::Error)]
pub enum JunitError {
    #[error("the JUnit report is not well-formed XML: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("the JUnit report has a malformed attribute: {0}")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("writing the JUnit report: {0}")]
    Write(#[from] std::io::Error),
    #[error(
        "linked tests of `{0}` are missing from the report, which has no <testsuites> \
         root to add a test suite to"
    )]
    NoRoot(String),
}

#[derive(Debug)]
pub struct Rewritten {
    pub xml: Vec<u8>,
    /// Test cases already in the report that gained the property.
    pub annotated: usize,
    /// Linked tests that were not in the report, added as skipped.
    pub added: usize,
}

/// What the first pass learns about the report.
#[derive(Default)]
struct Scan {
    present: HashSet<TestId>,
    already_annotated: HashSet<TestId>,
    suites: HashSet<String>,
}

pub fn rewrite(input: &[u8], links: &Links) -> Result<Rewritten, JunitError> {
    let scan = scan(input)?;
    let mut missing: BTreeMap<&str, Vec<(&TestId, &BTreeSet<String>)>> = BTreeMap::new();
    for (id, keys) in links {
        if !scan.present.contains(id) {
            missing.entry(&id.binary_id).or_default().push((id, keys));
        }
    }
    let total_missing: usize = missing.values().map(Vec::len).sum();

    let mut reader = Reader::from_reader(input);
    let mut writer = Writer::new(Vec::with_capacity(input.len() + links.len() * 96));
    let mut buf = Vec::new();
    let mut suites: Vec<String> = Vec::new();
    let mut filled: HashSet<&str> = HashSet::new();
    let mut saw_root = false;
    let mut annotated = 0;
    loop {
        let event = reader.read_event_into(&mut buf)?;
        match &event {
            Event::Eof => break,
            Event::Start(e) if e.local_name().as_ref() == b"testsuites" => {
                saw_root = true;
                writer.write_event(Event::Start(bump_counts(e, total_missing)?))?;
            }
            Event::Start(e) if e.local_name().as_ref() == b"testsuite" => {
                let name = attr(e, b"name")?.unwrap_or_default();
                let adding = match missing.get_key_value(name.as_str()) {
                    Some((suite, cases)) if filled.insert(suite) => cases.len(),
                    _ => 0,
                };
                suites.push(if adding > 0 { name } else { String::new() });
                writer.write_event(Event::Start(bump_counts(e, adding)?))?;
            }
            Event::End(e) if e.local_name().as_ref() == b"testsuite" => {
                if let Some(suite) = suites.pop().filter(|suite| !suite.is_empty()) {
                    for (id, keys) in &missing[suite.as_str()] {
                        write_skipped_case(&mut writer, id, keys)?;
                    }
                }
                writer.write_event(event.borrow())?;
            }
            Event::End(e) if e.local_name().as_ref() == b"testsuites" => {
                for (suite, cases) in &missing {
                    if !scan.suites.contains(*suite) {
                        write_suite(&mut writer, suite, cases)?;
                    }
                }
                writer.write_event(event.borrow())?;
            }
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"testcase" => {
                let keys = test_id(e)?
                    .filter(|id| !scan.already_annotated.contains(id))
                    .and_then(|id| links.get(&id));
                match keys {
                    Some(keys) => {
                        annotated += 1;
                        writer.write_event(Event::Start(e.borrow()))?;
                        write_properties(&mut writer, keys)?;
                        if matches!(event, Event::Empty(_)) {
                            writer.write_event(Event::End(end_of(e)))?;
                        }
                    }
                    None => writer.write_event(event.borrow())?,
                }
            }
            _ => writer.write_event(event.borrow())?,
        }
        buf.clear();
    }
    if !saw_root && let Some(suite) = missing.keys().find(|suite| !scan.suites.contains(**suite)) {
        return Err(JunitError::NoRoot(suite.to_string()));
    }
    Ok(Rewritten {
        xml: writer.into_inner(),
        annotated,
        added: total_missing,
    })
}

fn scan(input: &[u8]) -> Result<Scan, JunitError> {
    let mut scan = Scan::default();
    let mut reader = Reader::from_reader(input);
    let mut buf = Vec::new();
    // The test case being read, and whether we are in its <properties>.
    let mut case: Option<TestId> = None;
    let mut in_properties = false;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"testsuite" => {
                if let Some(name) = attr(&e, b"name")? {
                    scan.suites.insert(name);
                }
            }
            Event::Start(e) if e.local_name().as_ref() == b"testcase" => {
                case = test_id(&e)?;
                if let Some(id) = &case {
                    scan.present.insert(id.clone());
                }
            }
            Event::Empty(e) if e.local_name().as_ref() == b"testcase" => {
                if let Some(id) = test_id(&e)? {
                    scan.present.insert(id);
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"testcase" => {
                case = None;
                in_properties = false;
            }
            Event::Start(e) if e.local_name().as_ref() == b"properties" => {
                in_properties = case.is_some();
            }
            Event::End(e) if e.local_name().as_ref() == b"properties" => in_properties = false,
            Event::Start(e) | Event::Empty(e)
                if in_properties && e.local_name().as_ref() == b"property" =>
            {
                if attr(&e, b"name")?.as_deref() == Some(PROPERTY)
                    && let Some(id) = &case
                {
                    scan.already_annotated.insert(id.clone());
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(scan)
}

fn attr(e: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>, JunitError> {
    match e.try_get_attribute(name)? {
        Some(attribute) => Ok(Some(attribute.unescape_value()?.into_owned())),
        None => Ok(None),
    }
}

fn test_id(e: &BytesStart<'_>) -> Result<Option<TestId>, JunitError> {
    Ok(match (attr(e, b"classname")?, attr(e, b"name")?) {
        (Some(binary_id), Some(name)) => Some(TestId { binary_id, name }),
        _ => None,
    })
}

fn end_of<'a>(e: &'a BytesStart<'_>) -> BytesEnd<'a> {
    BytesEnd::new(String::from_utf8_lossy(e.name().as_ref()).into_owned())
}

/// `e` with `tests` and the skip count raised by `by`. The skip count is
/// `skipped`, or `disabled` in reports from older nextest versions, which
/// counted skips there; a report with neither gains `skipped`.
fn bump_counts(e: &BytesStart<'_>, by: usize) -> Result<BytesStart<'static>, JunitError> {
    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut out = BytesStart::new(name);
    if by == 0 {
        out.extend_attributes(e.attributes().filter_map(Result::ok));
        return Ok(out);
    }
    let has_skipped = e.try_get_attribute("skipped")?.is_some();
    let skip_attr: &[u8] = match (has_skipped, e.try_get_attribute("disabled")?.is_some()) {
        (false, true) => b"disabled",
        _ => b"skipped",
    };
    let mut wrote_skip = false;
    for attribute in e.attributes() {
        let attribute = attribute?;
        let key = attribute.key.as_ref();
        if key == b"tests" || key == skip_attr {
            wrote_skip |= key == skip_attr;
            let raised = attribute
                .unescape_value()?
                .trim()
                .parse::<usize>()
                .map(|n| (n + by).to_string());
            match raised {
                Ok(value) => out.push_attribute((key, value.as_bytes())),
                Err(_) => out.push_attribute(attribute),
            }
        } else {
            out.push_attribute(attribute);
        }
    }
    if !wrote_skip {
        out.push_attribute(("skipped", by.to_string().as_str()));
    }
    Ok(out)
}

fn write_properties(
    writer: &mut Writer<Vec<u8>>,
    keys: &BTreeSet<String>,
) -> Result<(), JunitError> {
    let value = keys
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",");
    writer.write_event(Event::Start(BytesStart::new("properties")))?;
    let mut property = BytesStart::new("property");
    property.push_attribute(("name", PROPERTY));
    property.push_attribute(("value", value.as_str()));
    writer.write_event(Event::Empty(property))?;
    writer.write_event(Event::End(BytesEnd::new("properties")))?;
    Ok(())
}

fn write_skipped_case(
    writer: &mut Writer<Vec<u8>>,
    id: &TestId,
    keys: &BTreeSet<String>,
) -> Result<(), JunitError> {
    let mut case = BytesStart::new("testcase");
    case.push_attribute(("name", id.name.as_str()));
    case.push_attribute(("classname", id.binary_id.as_str()));
    case.push_attribute(("time", "0"));
    writer.write_event(Event::Start(case))?;
    write_properties(writer, keys)?;
    let mut skipped = BytesStart::new("skipped");
    skipped.push_attribute(("message", SKIPPED_MESSAGE));
    writer.write_event(Event::Empty(skipped))?;
    writer.write_event(Event::End(BytesEnd::new("testcase")))?;
    Ok(())
}

fn write_suite(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    cases: &[(&TestId, &BTreeSet<String>)],
) -> Result<(), JunitError> {
    let count = cases.len().to_string();
    let mut suite = BytesStart::new("testsuite");
    suite.push_attribute(("name", name));
    suite.push_attribute(("tests", count.as_str()));
    suite.push_attribute(("skipped", count.as_str()));
    suite.push_attribute(("errors", "0"));
    suite.push_attribute(("failures", "0"));
    writer.write_event(Event::Start(suite))?;
    for (id, keys) in cases {
        write_skipped_case(writer, id, keys)?;
    }
    writer.write_event(Event::End(BytesEnd::new("testsuite")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// nextest's JUnit report, as 0.9.138 writes it.
    const REPORT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="3" failures="0" errors="0" time="0.004">
    <testsuite name="krate::it" tests="2" disabled="0" errors="0" failures="0">
        <testcase name="nested::deep" classname="krate::it" time="0.003">
        </testcase>
        <testcase name="a &amp; b" classname="krate::it" time="0.004"/>
    </testsuite>
    <testsuite name="krate" tests="1" skipped="0" errors="0" failures="0">
        <testcase name="tests::adds" classname="krate" time="0.003">
            <failure message="boom"/>
        </testcase>
    </testsuite>
</testsuites>
"#;

    fn links(entries: &[(&str, &str, &[&str])]) -> Links {
        entries
            .iter()
            .map(|(binary_id, name, keys)| {
                (
                    TestId {
                        binary_id: binary_id.to_string(),
                        name: name.to_string(),
                    },
                    keys.iter().map(|k| k.to_string()).collect(),
                )
            })
            .collect()
    }

    /// A small reader for the rewritten report: every case's identity, its
    /// `requirements` value, and whether it is skipped; and each suite's
    /// counts.
    fn cases(xml: &[u8]) -> Vec<(String, String, Option<String>, bool)> {
        let mut out = Vec::new();
        let mut reader = Reader::from_reader(xml);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf).unwrap() {
                Event::Eof => break,
                Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                    b"testcase" => {
                        let id = test_id(&e).unwrap().unwrap();
                        out.push((id.binary_id, id.name, None, false));
                    }
                    b"property" => {
                        let value = attr(&e, b"value").unwrap();
                        out.last_mut().unwrap().2 = value;
                    }
                    b"skipped" => out.last_mut().unwrap().3 = true,
                    _ => {}
                },
                _ => {}
            }
            buf.clear();
        }
        out
    }

    fn counts(xml: &[u8], element: &[u8]) -> Vec<(Option<String>, Option<String>, Option<String>)> {
        let mut out = Vec::new();
        let mut reader = Reader::from_reader(xml);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf).unwrap() {
                Event::Eof => break,
                Event::Start(e) if e.local_name().as_ref() == element => out.push((
                    attr(&e, b"tests").unwrap(),
                    attr(&e, b"skipped").unwrap(),
                    attr(&e, b"disabled").unwrap(),
                )),
                _ => {}
            }
            buf.clear();
        }
        out
    }

    fn s(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    #[test]
    fn linked_cases_gain_the_property_and_missing_ones_are_added_skipped() {
        let links = links(&[
            ("krate::it", "nested::deep", &["REQ-1.1", "REQ-2"]),
            ("krate::it", "a & b", &["REQ-3"]),
            ("krate::it", "ignored", &["REQ-4"]),
            ("krate", "tests::adds", &["REQ-5"]),
            ("krate::other", "never_run", &["REQ-6"]),
        ]);
        let out = rewrite(REPORT.as_bytes(), &links).unwrap();
        assert_eq!((out.annotated, out.added), (3, 2));
        let text = String::from_utf8(out.xml.clone()).unwrap();
        assert_eq!(
            cases(&out.xml),
            [
                (
                    s("krate::it").unwrap(),
                    s("nested::deep").unwrap(),
                    s("REQ-1.1,REQ-2"),
                    false
                ),
                (
                    s("krate::it").unwrap(),
                    s("a & b").unwrap(),
                    s("REQ-3"),
                    false
                ),
                (
                    s("krate::it").unwrap(),
                    s("ignored").unwrap(),
                    s("REQ-4"),
                    true
                ),
                (
                    s("krate").unwrap(),
                    s("tests::adds").unwrap(),
                    s("REQ-5"),
                    false
                ),
                (
                    s("krate::other").unwrap(),
                    s("never_run").unwrap(),
                    s("REQ-6"),
                    true
                ),
            ],
            "{text}"
        );
        assert!(text.contains(r#"<failure message="boom"/>"#), "{text}");
        assert_eq!(counts(&out.xml, b"testsuites"), [(s("5"), s("2"), None)]);
        assert_eq!(
            counts(&out.xml, b"testsuite"),
            [
                (s("3"), None, s("1")),
                (s("1"), s("0"), None),
                (s("1"), s("1"), None),
            ],
            "{text}"
        );
    }

    #[test]
    fn rewriting_twice_changes_nothing() {
        let links = links(&[
            ("krate::it", "nested::deep", &["REQ-1"]),
            ("krate::it", "ignored", &["REQ-4"]),
        ]);
        let once = rewrite(REPORT.as_bytes(), &links).unwrap();
        let twice = rewrite(&once.xml, &links).unwrap();
        assert_eq!((twice.annotated, twice.added), (0, 0));
        assert_eq!(twice.xml, once.xml);
    }

    #[test]
    fn no_links_is_a_byte_identical_copy() {
        let out = rewrite(REPORT.as_bytes(), &Links::new()).unwrap();
        assert_eq!(String::from_utf8(out.xml).unwrap(), REPORT);
    }

    #[test]
    fn a_report_without_a_root_cannot_gain_a_suite() {
        let lone = r#"<testsuite name="krate" tests="0"></testsuite>"#;
        let error = rewrite(lone.as_bytes(), &links(&[("other", "t", &["REQ-1"])])).unwrap_err();
        assert!(matches!(error, JunitError::NoRoot(suite) if suite == "other"));
        let fine = rewrite(lone.as_bytes(), &links(&[("krate", "t", &["REQ-1"])])).unwrap();
        assert_eq!(fine.added, 1);
    }

    #[test]
    fn malformed_xml_is_an_error() {
        assert!(rewrite(b"<testsuites><testsuite></testsuites>", &Links::new()).is_err());
    }

    /// A report the size of a large workspace's: every case annotated, one
    /// suite per binary, and missing tests spread across suites.
    #[test]
    fn rewrites_a_large_report() {
        const SUITES: usize = 100;
        const PER_SUITE: usize = 500;
        let mut xml =
            String::from(r#"<testsuites name="r" tests="50000" failures="0" errors="0">"#);
        let mut entries = Vec::new();
        for suite in 0..SUITES {
            xml.push_str(&format!(
                r#"<testsuite name="b{suite}" tests="{PER_SUITE}" skipped="0" errors="0" failures="0">"#
            ));
            for case in 0..PER_SUITE {
                xml.push_str(&format!(
                    r#"<testcase name="t{case}" classname="b{suite}" time="0.001"/>"#
                ));
                entries.push((format!("b{suite}"), format!("t{case}")));
            }
            entries.push((format!("b{suite}"), "missing".to_string()));
            xml.push_str("</testsuite>");
        }
        xml.push_str("</testsuites>");
        let links: Links = entries
            .into_iter()
            .map(|(binary_id, name)| {
                (
                    TestId { binary_id, name },
                    BTreeSet::from(["REQ-1".to_string()]),
                )
            })
            .collect();
        let out = rewrite(xml.as_bytes(), &links).unwrap();
        assert_eq!((out.annotated, out.added), (SUITES * PER_SUITE, SUITES));
        let cases = cases(&out.xml);
        assert_eq!(cases.len(), SUITES * (PER_SUITE + 1));
        assert!(cases.iter().all(|case| case.2.as_deref() == Some("REQ-1")));
        assert_eq!(cases.iter().filter(|case| case.3).count(), SUITES);
    }
}
