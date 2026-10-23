//! A baseline of the functions that may stay over a maximum.
//!
//! A project adopting `--max-complexity` or `--max-crap` usually has functions
//! that are already over it. A baseline records them, so the gate fails only for
//! a new function over the maximum or one that has got worse. Fix a baselined
//! function and it drops out of the gate by itself.
//!
//! A function is identified by its file and its name, not its line, which moves
//! with every edit above it. Functions that share a name in one file (and
//! closures, which have none) share one entry holding their highest scores, so
//! a second one that is no worse than the first is not caught.
//!
//! The file is written and read by this tool only, so it is a small binary
//! format rather than text. All integers are LEB128 varints:
//!
//! ```text
//! "JCIB"  version:u8  rules:u8
//! file count, then each file name (length, UTF-8 bytes), in ascending order
//! entry count, then each entry in ascending order of (file, name):
//!     file index, function name (length, UTF-8 bytes), complexity,
//!     CRAP in tenths rounded up, plus one (0 when coverage was unknown)
//! CRC-32 of everything above, 4 bytes little-endian
//! ```
//!
//! The checksum is checked before anything else is read, and a file that is
//! truncated, altered, out of order or written for the other rule set is
//! refused, so a damaged baseline can never silently allow a function. The
//! output depends only on the entries, so regenerating an unchanged baseline
//! gives the same bytes.

use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, anyhow, bail};

use crate::complexity::{counter::Rules, metrics::FunctionMetrics};

const MAGIC: &[u8; 4] = b"JCIB";
const VERSION: u8 = 1;
/// Magic, version and rules, before the body.
const HEADER_LEN: usize = MAGIC.len() + 2;
const CRC_LEN: usize = 4;

/// The name closures are recorded under.
const CLOSURE: &str = "<closure>";

/// The scores a baselined function may not exceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Allowance {
    complexity: u32,
    /// CRAP in tenths, rounded up. `None` when its coverage was unknown, which
    /// allows any CRAP.
    crap: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Baseline {
    rules: Rules,
    entries: BTreeMap<(String, String), Allowance>,
}

fn key(f: &FunctionMetrics) -> (String, String) {
    let name = if f.name.is_empty() { CLOSURE } else { &f.name };
    (f.file.clone(), name.to_string())
}

/// CRAP in whole tenths, rounded up, so a function is never failed for digits
/// the baseline did not keep. Both the recording and the comparison use it.
fn tenths(crap: f64) -> u64 {
    (crap * 10.0).ceil() as u64
}

fn rules_tag(rules: Rules) -> u8 {
    match rules {
        Rules::Rca => 0,
        Rules::Mccabe => 1,
    }
}

fn rules_from_tag(tag: u8) -> Option<Rules> {
    match tag {
        0 => Some(Rules::Rca),
        1 => Some(Rules::Mccabe),
        _ => None,
    }
}

/// CRC-32 (IEEE 802.3, as in zip and PNG).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn put_varint(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

/// A cursor over the body that reports a short read rather than panicking.
struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: u64) -> Result<&'a [u8]> {
        let n = usize::try_from(n).ok().filter(|n| *n <= self.buf.len());
        let n = n.ok_or_else(|| anyhow!("it ends in the middle of a value"))?;
        let (head, rest) = self.buf.split_at(n);
        self.buf = rest;
        Ok(head)
    }

    fn varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.bytes(1)?[0];
            let bits = u64::from(byte & 0x7f);
            if shift == 63 && bits > 1 {
                bail!("a number is too large");
            }
            value |= bits << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        bail!("a number is too long")
    }

    fn string(&mut self) -> Result<String> {
        let len = self.varint()?;
        String::from_utf8(self.bytes(len)?.to_vec()).map_err(|_| anyhow!("a name is not UTF-8"))
    }

    fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

impl Baseline {
    /// A baseline allowing exactly these functions as they are, scored with
    /// `rules`.
    pub(crate) fn from_functions<'a>(
        functions: impl IntoIterator<Item = &'a FunctionMetrics>,
        rules: Rules,
    ) -> Self {
        let mut entries: BTreeMap<(String, String), Allowance> = BTreeMap::new();
        for f in functions {
            let crap = f.crap.map(tenths);
            entries
                .entry(key(f))
                .and_modify(|a| {
                    a.complexity = a.complexity.max(f.complexity);
                    a.crap = match (a.crap, crap) {
                        (Some(x), Some(y)) => Some(x.max(y)),
                        (x, y) => x.or(y),
                    };
                })
                .or_insert(Allowance {
                    complexity: f.complexity,
                    crap,
                });
        }
        Self { rules, entries }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The rule set the scores were made with.
    pub(crate) fn rules(&self) -> Rules {
        self.rules
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        // Entries are sorted by file, so each file name is stored once.
        let mut files: Vec<&str> = Vec::new();
        for (file, _) in self.entries.keys() {
            if files.last() != Some(&file.as_str()) {
                files.push(file);
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(rules_tag(self.rules));
        put_varint(&mut out, files.len() as u64);
        for file in &files {
            put_str(&mut out, file);
        }
        put_varint(&mut out, self.entries.len() as u64);
        let mut index = 0;
        for ((file, name), a) in &self.entries {
            while files[index] != file {
                index += 1;
            }
            put_varint(&mut out, index as u64);
            put_str(&mut out, name);
            put_varint(&mut out, u64::from(a.complexity));
            put_varint(&mut out, a.crap.map_or(0, |c| c.saturating_add(1)));
        }
        let crc = crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + CRC_LEN || &bytes[..MAGIC.len()] != MAGIC {
            bail!("not a jci-coverage baseline");
        }
        let version = bytes[MAGIC.len()];
        if version != VERSION {
            bail!(
                "baseline format version {version} is not supported (this is version {VERSION}); regenerate it with --write-baseline"
            );
        }
        let (body, crc) = bytes.split_at(bytes.len() - CRC_LEN);
        if crc32(body).to_le_bytes() != crc {
            bail!("baseline is corrupt (checksum mismatch); regenerate it with --write-baseline");
        }
        Self::decode_body(body).map_err(|e| anyhow!("baseline is malformed: {e}"))
    }

    fn decode_body(body: &[u8]) -> Result<Self> {
        let rules = rules_from_tag(body[HEADER_LEN - 1])
            .ok_or_else(|| anyhow!("unknown rule set {}", body[HEADER_LEN - 1]))?;
        let mut r = Reader {
            buf: &body[HEADER_LEN..],
        };
        let mut files: Vec<String> = Vec::new();
        for _ in 0..r.varint()? {
            let file = r.string()?;
            if files.last().is_some_and(|last| *last >= file) {
                bail!("files are not in order");
            }
            files.push(file);
        }
        let mut entries: BTreeMap<(String, String), Allowance> = BTreeMap::new();
        for _ in 0..r.varint()? {
            let index = r.varint()?;
            let file = usize::try_from(index)
                .ok()
                .and_then(|i| files.get(i))
                .ok_or_else(|| anyhow!("an entry names file {index}, which does not exist"))?;
            let name = r.string()?;
            let complexity =
                u32::try_from(r.varint()?).map_err(|_| anyhow!("a complexity is too large"))?;
            let crap = r.varint()?.checked_sub(1);
            let entry = (file.clone(), name);
            if entries
                .last_key_value()
                .is_some_and(|(last, _)| *last >= entry)
            {
                bail!("entries are not in order");
            }
            entries.insert(entry, Allowance { complexity, crap });
        }
        if !r.is_empty() {
            bail!("there is data after the last entry");
        }
        Ok(Self { rules, entries })
    }

    /// Whether `f` is in the baseline and no worse than it was recorded.
    pub(crate) fn allows(&self, f: &FunctionMetrics) -> bool {
        let Some(a) = self.entries.get(&key(f)) else {
            return false;
        };
        f.complexity <= a.complexity
            && match (f.crap, a.crap) {
                (Some(now), Some(then)) => tenths(now) <= then,
                _ => true,
            }
    }

    /// How many entries match none of the functions currently over a maximum:
    /// the function is gone or has dropped under it.
    pub(crate) fn unused<'a>(&self, over: impl IntoIterator<Item = &'a FunctionMetrics>) -> usize {
        let used: HashSet<_> = over.into_iter().map(key).collect();
        self.entries.keys().filter(|k| !used.contains(*k)).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn func(file: &str, name: &str, complexity: u32, crap: Option<f64>) -> FunctionMetrics {
        FunctionMetrics {
            file: file.into(),
            name: name.into(),
            line: 1,
            complexity,
            lines: 4,
            covered: 0,
            crap,
            is_test: false,
        }
    }

    fn baseline(functions: &[FunctionMetrics]) -> Baseline {
        Baseline::from_functions(functions, Rules::Rca)
    }

    /// A body with a valid checksum, so only its structure is under test.
    fn seal(mut body: Vec<u8>) -> Vec<u8> {
        let crc = crc32(&body);
        body.extend_from_slice(&crc.to_le_bytes());
        body
    }

    fn header(rules: u8) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend([VERSION, rules]);
        v
    }

    fn sample() -> Baseline {
        baseline(&[
            func("src/b.rs", "zed", 20, Some(41.26)),
            func("src/a.rs", "alpha", 12, None),
            func("src/a.rs", "beta", 300, Some(90_000.0)),
            func("src/ünï.rs", "", 11, Some(0.0)),
        ])
    }

    #[test]
    fn the_checksum_is_the_standard_crc32() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_baseline_round_trips() {
        let b = sample();
        assert_eq!(b.len(), 4);
        let bytes = b.encode();
        assert_eq!(&bytes[..4], MAGIC);
        assert_eq!(Baseline::decode(&bytes).expect("decodes"), b);
        let mccabe = Baseline::from_functions(&[func("a.rs", "f", 1, None)], Rules::Mccabe);
        assert_eq!(
            Baseline::decode(&mccabe.encode()).expect("decodes").rules(),
            Rules::Mccabe
        );
        let empty = baseline(&[]);
        assert_eq!(Baseline::decode(&empty.encode()).expect("decodes"), empty);
    }

    #[test]
    fn the_bytes_depend_only_on_the_entries() {
        let forward = baseline(&[func("a.rs", "x", 1, None), func("b.rs", "y", 2, Some(3.0))]);
        let backward = baseline(&[func("b.rs", "y", 2, Some(3.0)), func("a.rs", "x", 1, None)]);
        assert_eq!(forward.encode(), backward.encode());
        assert_eq!(forward.encode(), forward.encode());
    }

    #[test]
    fn it_is_small_and_names_each_file_once() {
        let file = "crates/jci-coverage/src/a/long/path/module.rs";
        let functions: Vec<_> = (0..50)
            .map(|n| func(file, &format!("f{n:02}"), 10 + n, Some(40.0)))
            .collect();
        let bytes = baseline(&functions).encode();
        let mut occurrences = 0;
        for window in bytes.windows(file.len()) {
            occurrences += usize::from(window == file.as_bytes());
        }
        assert_eq!(occurrences, 1);
        // Per entry: index, name length, 3 name bytes, complexity, CRAP code.
        assert!(
            bytes.len() < 50 * 8 + file.len() + 16,
            "{} bytes",
            bytes.len()
        );
    }

    #[test]
    fn anything_that_is_not_a_baseline_is_refused() {
        for bytes in [
            &b""[..],
            b"JCIB",
            b"hello world, not a baseline",
            b"\0\0\0\0\0\0\0\0\0\0",
        ] {
            let err = Baseline::decode(bytes).unwrap_err().to_string();
            assert!(err.contains("not a jci-coverage baseline"), "{err}");
        }
    }

    #[test]
    fn another_format_version_is_refused() {
        let mut bytes = sample().encode();
        bytes[4] = VERSION + 1;
        let err = Baseline::decode(&bytes).unwrap_err().to_string();
        assert!(err.contains("version 2 is not supported"), "{err}");
    }

    #[test]
    fn every_truncation_is_refused() {
        let bytes = sample().encode();
        for len in 0..bytes.len() {
            assert!(Baseline::decode(&bytes[..len]).is_err(), "length {len}");
        }
    }

    #[test]
    fn every_single_altered_byte_is_refused() {
        let bytes = sample().encode();
        for at in 0..bytes.len() {
            for flip in [0x01, 0x80, 0xff] {
                let mut altered = bytes.clone();
                altered[at] ^= flip;
                assert!(Baseline::decode(&altered).is_err(), "byte {at} ^ {flip:#x}");
            }
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(Baseline::decode(&extended).is_err(), "trailing byte");
    }

    #[test]
    fn a_damaged_file_says_it_is_corrupt() {
        let mut bytes = sample().encode();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0x10;
        let err = Baseline::decode(&bytes).unwrap_err().to_string();
        assert!(
            err.contains("corrupt") && err.contains("--write-baseline"),
            "{err}"
        );
    }

    #[test]
    fn a_checksummed_body_with_a_bad_structure_is_refused() {
        let with = |body: &[u8]| {
            let mut v = header(0);
            v.extend_from_slice(body);
            Baseline::decode(&seal(v)).unwrap_err().to_string()
        };
        // 0 files, 1 entry naming file 0.
        assert!(with(&[0, 1, 0, 1, b'f', 1, 1]).contains("file 0, which does not exist"));
        // 1 file "a", 2 entries out of order.
        assert!(
            with(&[1, 1, b'a', 2, 0, 1, b'b', 1, 0, 0, 1, b'a', 1, 0]).contains("not in order")
        );
        // 1 file "a", the same entry twice.
        assert!(
            with(&[1, 1, b'a', 2, 0, 1, b'b', 1, 0, 0, 1, b'b', 1, 0]).contains("not in order")
        );
        // 2 files out of order.
        assert!(with(&[2, 1, b'b', 1, b'a', 0]).contains("files are not in order"));
        // A name that is not UTF-8.
        assert!(with(&[1, 1, b'a', 1, 0, 1, 0xff, 1, 0]).contains("not UTF-8"));
        // A name longer than the data left.
        assert!(with(&[1, 1, b'a', 1, 0, 9, b'x']).contains("middle of a value"));
        // A count promising more entries than there are.
        assert!(with(&[0, 5]).contains("middle of a value"));
        // A number that never ends, and one that overflows.
        assert!(with(&[0x80; 11]).contains("too long"));
        let mut overflow = vec![0xff; 9];
        overflow.push(0x7f);
        assert!(with(&overflow).contains("too large"));
        // A complexity beyond u32.
        assert!(
            with(&[1, 1, b'a', 1, 0, 1, b'f', 0x80, 0x80, 0x80, 0x80, 0x10, 0])
                .contains("complexity is too large")
        );
        // Data after the last entry.
        assert!(with(&[0, 0, 0]).contains("after the last entry"));
        // An unknown rule set.
        let mut v = header(9);
        v.extend([0, 0]);
        let err = Baseline::decode(&seal(v)).unwrap_err().to_string();
        assert!(err.contains("unknown rule set 9"), "{err}");
    }

    #[test]
    fn a_function_no_worse_than_its_entry_is_allowed() {
        let b = baseline(&[func("src/a.rs", "f", 15, Some(30.04))]);
        assert!(
            b.allows(&func("src/a.rs", "f", 15, Some(30.04))),
            "as recorded"
        );
        assert!(b.allows(&func("src/a.rs", "f", 9, Some(2.0))), "better");
        assert!(
            b.allows(&func("src/a.rs", "f", 15, None)),
            "coverage now unknown"
        );
        assert!(
            !b.allows(&func("src/a.rs", "f", 16, Some(30.0))),
            "more complex"
        );
        assert!(
            !b.allows(&func("src/a.rs", "f", 15, Some(31.0))),
            "worse CRAP"
        );
    }

    #[test]
    fn a_function_recorded_and_read_back_is_never_failed_for_rounding() {
        for crap in [29.2, 39.2, 0.1, 0.3, 18.0, 41.26, 1e6 + 0.05] {
            let f = func("src/a.rs", "f", 10, Some(crap));
            let read = Baseline::decode(&baseline(std::slice::from_ref(&f)).encode()).expect("ok");
            assert!(read.allows(&f), "{crap}");
        }
    }

    #[test]
    fn a_function_not_in_the_baseline_is_not_allowed() {
        let b = baseline(&[func("src/a.rs", "f", 15, Some(30.0))]);
        assert!(
            !b.allows(&func("src/a.rs", "g", 1, Some(1.0))),
            "other name"
        );
        assert!(
            !b.allows(&func("src/b.rs", "f", 1, Some(1.0))),
            "other file"
        );
    }

    #[test]
    fn an_entry_without_a_known_crap_allows_any_crap() {
        let b = baseline(&[func("src/a.rs", "f", 15, None)]);
        assert!(b.allows(&func("src/a.rs", "f", 15, Some(500.0))));
        assert!(!b.allows(&func("src/a.rs", "f", 16, Some(1.0))));
    }

    #[test]
    fn functions_sharing_a_name_share_their_highest_scores() {
        let b = baseline(&[
            func("src/a.rs", "new", 12, Some(20.0)),
            func("src/a.rs", "new", 18, None),
            func("src/a.rs", "new", 5, Some(33.0)),
        ]);
        assert_eq!(b.len(), 1);
        assert!(b.allows(&func("src/a.rs", "new", 18, Some(33.0))));
        assert!(!b.allows(&func("src/a.rs", "new", 19, Some(1.0))));
    }

    #[test]
    fn closures_are_recorded_under_one_name_per_file() {
        let b = baseline(&[func("src/a.rs", "", 11, Some(12.0))]);
        assert!(b.allows(&func("src/a.rs", "", 11, Some(12.0))));
        assert!(b.allows(&func("src/a.rs", "<closure>", 11, Some(12.0))));
    }

    #[test]
    fn entries_for_functions_no_longer_over_a_maximum_are_counted_as_unused() {
        let b = baseline(&[
            func("src/a.rs", "kept", 15, Some(30.0)),
            func("src/a.rs", "fixed", 15, Some(30.0)),
            func("src/b.rs", "gone", 15, Some(30.0)),
        ]);
        let still_over = [func("src/a.rs", "kept", 14, Some(20.0))];
        assert_eq!(b.unused(&still_over), 2);
        assert_eq!(b.unused(&[]), 3);
    }
}
