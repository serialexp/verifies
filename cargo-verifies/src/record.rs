//! Reading `#[verifies]` records out of a compiled test binary. The layout is
//! specified in `verifies/FORMAT.md`.

use std::fs::File;
use std::path::Path;

use object::read::ReadCache;
use object::{Object, ObjectSection};

use crate::key::is_key;

const MAGIC: &[u8; 4] = b"VRFY";
const HEADER_LEN: usize = 9;
/// The only record version this reader understands.
pub const VERSION: u8 = 1;

/// One annotated test: its compiler-given path and the keys it verifies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// `module_path!()` plus the function name; the first segment is the
    /// crate.
    pub path: String,
    pub keys: Vec<String>,
}

impl Record {
    /// The test's name as libtest and nextest report it: the path without
    /// its crate segment.
    pub fn test_name(&self) -> Option<&str> {
        self.path.split_once("::").map(|(_, rest)| rest)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("record at byte {offset} is cut short")]
    Truncated { offset: usize },
    #[error("bytes at {offset} are not a record (bad magic)")]
    BadMagic { offset: usize },
    #[error(
        "record version {version} is newer than this cargo-verifies understands \
         (version {VERSION}); upgrade cargo-verifies"
    )]
    UnsupportedVersion { version: u8 },
    #[error("record at byte {offset} is not UTF-8")]
    NotUtf8 { offset: usize },
    #[error("record at byte {offset} has no path/keys separator")]
    MissingSeparator { offset: usize },
    #[error("`{path}` declares no keys")]
    NoKeys { path: String },
    #[error("`{path}` declares `{key}`, which is not a requirement key (REQ-<n> or REQ-<n>.<m>)")]
    MalformedKey { path: String, key: String },
}

/// Every record in a section's bytes, in order. Zero bytes between records
/// are linker padding and are skipped.
pub fn parse_records(section: &[u8]) -> Result<Vec<Record>, RecordError> {
    let mut records = Vec::new();
    let mut offset = 0;
    while offset < section.len() {
        if section[offset] == 0 {
            offset += 1;
            continue;
        }
        let header = section
            .get(offset..offset + HEADER_LEN)
            .ok_or(RecordError::Truncated { offset })?;
        if &header[..4] != MAGIC {
            return Err(RecordError::BadMagic { offset });
        }
        if header[4] != VERSION {
            return Err(RecordError::UnsupportedVersion { version: header[4] });
        }
        let length = u32::from_le_bytes([header[5], header[6], header[7], header[8]]) as usize;
        let start = offset + HEADER_LEN;
        let payload = start
            .checked_add(length)
            .and_then(|end| section.get(start..end))
            .ok_or(RecordError::Truncated { offset })?;
        records.push(parse_payload(payload, offset)?);
        offset = start + length;
    }
    Ok(records)
}

fn parse_payload(payload: &[u8], offset: usize) -> Result<Record, RecordError> {
    let payload = std::str::from_utf8(payload).map_err(|_| RecordError::NotUtf8 { offset })?;
    let (path, keys) = payload
        .split_once('\0')
        .ok_or(RecordError::MissingSeparator { offset })?;
    if keys.is_empty() {
        return Err(RecordError::NoKeys {
            path: path.to_string(),
        });
    }
    let keys: Vec<String> = keys.split(',').map(str::to_string).collect();
    if let Some(key) = keys.iter().find(|key| !is_key(key)) {
        return Err(RecordError::MalformedKey {
            path: path.to_string(),
            key: key.clone(),
        });
    }
    Ok(Record {
        path: path.to_string(),
        keys,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{}: {source}", path.display())]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("{}: not a readable object file: {source}", path.display())]
    Object {
        path: std::path::PathBuf,
        source: object::Error,
    },
    #[error("{}: {source}", path.display())]
    Record {
        path: std::path::PathBuf,
        source: RecordError,
    },
}

/// The records in one test binary (ELF, Mach-O or PE). Reads only the
/// headers and the record section, never the whole file.
pub fn read_binary(path: &Path) -> Result<Vec<Record>, ReadError> {
    let file = File::open(path).map_err(|source| ReadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let cache = ReadCache::new(file);
    let object = object::File::parse(&cache).map_err(|source| ReadError::Object {
        path: path.to_path_buf(),
        source,
    })?;
    let mut records = Vec::new();
    for section in object.sections() {
        if !is_record_section(&section) {
            continue;
        }
        let data = section.data().map_err(|source| ReadError::Object {
            path: path.to_path_buf(),
            source,
        })?;
        records.extend(parse_records(data).map_err(|source| ReadError::Record {
            path: path.to_path_buf(),
            source,
        })?);
    }
    Ok(records)
}

fn is_record_section<'data, S: ObjectSection<'data>>(section: &S) -> bool {
    match section.name() {
        Ok("verifies") => true,
        Ok("__verifies") => matches!(section.segment_name(), Ok(Some("__DATA"))),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(version: u8, payload: &str) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.push(version);
        bytes.extend((payload.len() as u32).to_le_bytes());
        bytes.extend(payload.as_bytes());
        bytes
    }

    #[test]
    fn records_parse_across_padding() {
        let mut section = vec![0, 0];
        section.extend(record(1, "krate::tests::a\0REQ-1.1,REQ-2"));
        section.extend([0; 7]);
        section.extend(record(1, "it::b\0REQ-3"));
        let records = parse_records(&section).unwrap();
        assert_eq!(
            records,
            [
                Record {
                    path: "krate::tests::a".into(),
                    keys: vec!["REQ-1.1".into(), "REQ-2".into()],
                },
                Record {
                    path: "it::b".into(),
                    keys: vec!["REQ-3".into()],
                },
            ]
        );
        assert_eq!(records[0].test_name(), Some("tests::a"));
        assert_eq!(records[1].test_name(), Some("b"));
        assert!(parse_records(&[0, 0, 0]).unwrap().is_empty());
    }

    #[test]
    fn bad_records_are_errors() {
        let error = |bytes: &[u8]| parse_records(bytes).unwrap_err().to_string();
        assert!(error(&record(2, "a::b\0REQ-1")).contains("version 2"));
        assert!(error(b"VRFX\x01\0\0\0\0").contains("bad magic"));
        assert!(error(&record(1, "a::b\0REQ-1")[..12]).contains("cut short"));
        assert!(error(b"VRFY\x01").contains("cut short"));
        assert!(error(&record(1, "a::b REQ-1")).contains("separator"));
        assert!(error(&record(1, "a::b\0")).contains("no keys"));
        assert!(error(&record(1, "a::b\0REQ-1,req-2")).contains("`req-2`"));
        let mut huge = MAGIC.to_vec();
        huge.push(1);
        huge.extend(u32::MAX.to_le_bytes());
        assert!(error(&huge).contains("cut short"));
    }
}
