//! A baseline of the functions that may stay over a maximum.
//!
//! A project adopting `--max-complexity` or `--max-crap` usually has functions
//! that are already over it. A baseline records them, so the gate fails only for
//! a new function over the maximum or one that has got worse. Fix a baselined
//! function and it drops out of the gate by itself.
//!
//! "Worse" is judged only in a score the function is at or over its maximum in:
//! complexity if it is at or over `--max-complexity`, CRAP if it is at or over
//! `--max-crap`, both if it is at or over both. A score still under its own
//! maximum may move (CRAP follows coverage, which differs by a line between
//! runs) without failing a function accepted for the other; once it reaches the
//! maximum it is judged against the recorded value.
//!
//! Every baselined function whose scores differ from the recorded ones is
//! reported by [`Baseline::movements`], without failing the gate: a degradation
//! within what the baseline accepts is a prompt to rework the function, and an
//! improvement shows where the baseline can be recorded again to hold the gain.
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

use crate::complexity::{
    counter::Rules,
    metrics::{FunctionMetrics, Risk, Thresholds, risky},
};

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
    /// CRAP in tenths, rounded up. `None` when its coverage was unknown, so no
    /// CRAP was accepted.
    crap: Option<u64>,
}

/// The scores a baselined function was recorded with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) complexity: u32,
    /// `None` when its coverage was unknown.
    pub(crate) crap: Option<f64>,
}

/// How a function compares with its baseline entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Standing {
    /// Not in the baseline: a new function.
    New,
    /// In the baseline and no worse than recorded.
    Allowed,
    /// In the baseline but worse, in the scores flagged.
    Worse {
        complexity: bool,
        crap: bool,
        recorded: Recorded,
    },
}

/// Which way a score has gone from the one recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    Worse,
    Better,
}

/// A baselined function whose scores differ from the recorded ones: reported,
/// never failed, since the gate judges only a limit the function is over.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Movement<'a> {
    /// The function; of several with one name in a file, the most complex.
    pub(crate) function: &'a FunctionMetrics,
    /// What it was recorded with.
    pub(crate) recorded: Recorded,
    /// Its highest scores now.
    pub(crate) now: Recorded,
    /// `None` when the score is as recorded, or either value is unknown.
    pub(crate) complexity: Option<Direction>,
    pub(crate) crap: Option<Direction>,
}

impl Movement<'_> {
    /// Any score worse than recorded, whatever else improved.
    pub(crate) fn degraded(&self) -> bool {
        [self.complexity, self.crap].contains(&Some(Direction::Worse))
    }

    /// Some score better than recorded and none worse.
    pub(crate) fn improved(&self) -> bool {
        !self.degraded() && [self.complexity, self.crap].contains(&Some(Direction::Better))
    }

    /// The complexity as recorded and now, and which way it went.
    pub(crate) fn complexity_text(&self) -> String {
        moved_text(
            Some(self.now.complexity.to_string()),
            Some(self.recorded.complexity.to_string()),
            self.complexity,
        )
    }

    /// The CRAP as recorded and now, and which way it went. It is shown as it is
    /// compared, in tenths rounded up, so a movement never reads as `29.2 → 29.2`.
    pub(crate) fn crap_text(&self) -> String {
        let tenth = |c: f64| format!("{c:.1}");
        moved_text(
            self.now.crap.map(tenth),
            self.recorded.crap.map(tenth),
            self.crap,
        )
    }
}

/// A score moved: `then → now (worse)` or `(better)`, `now (same)`, `n/a` when
/// there is none now, `now, none recorded` when none was recorded.
fn moved_text(now: Option<String>, then: Option<String>, direction: Option<Direction>) -> String {
    match (now, then, direction) {
        (None, _, _) => "n/a".to_string(),
        (Some(now), None, _) => format!("{now}, none recorded"),
        (Some(now), Some(_), None) => format!("{now} (same)"),
        (Some(now), Some(then), Some(Direction::Worse)) => format!("{then} → {now} (worse)"),
        (Some(now), Some(then), Some(Direction::Better)) => format!("{then} → {now} (better)"),
    }
}

fn direction<T: PartialOrd>(now: T, then: T) -> Option<Direction> {
    if now > then {
        Some(Direction::Worse)
    } else if now < then {
        Some(Direction::Better)
    } else {
        None
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Baseline {
    rules: Rules,
    entries: BTreeMap<(String, String), Allowance>,
}

/// A function's identity: its file with `/` separators, and its name.
fn key(f: &FunctionMetrics) -> (String, String) {
    let name = if f.name.is_empty() { CLOSURE } else { &f.name };
    (f.file.replace('\\', "/"), name.to_string())
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

    /// Where the function stands against the baseline, and, if it is worse, in
    /// what.
    ///
    /// Only a score the function is at or over its limit in is judged: a function
    /// at or over the complexity limit alone is not worse for a CRAP that moved
    /// while still under its limit, and the reverse. A score that has reached its
    /// limit is compared with the value recorded, so a CRAP recorded at 29.2
    /// under a limit of 30 is worse at 30.2.
    pub(crate) fn standing(&self, risk: &Risk<'_>) -> Standing {
        let f = risk.function;
        let Some(a) = self.entries.get(&key(f)) else {
            return Standing::New;
        };
        let complexity = risk.complexity && f.complexity > a.complexity;
        let crap = risk.crap
            && match (f.crap, a.crap) {
                (Some(now), Some(then)) => tenths(now) > then,
                // CRAP was never accepted for it, so a known one is new.
                (Some(_), None) => true,
                // Coverage data is missing now; complexity alone is judged.
                (None, _) => false,
            };
        if !complexity && !crap {
            return Standing::Allowed;
        }
        Standing::Worse {
            complexity,
            crap,
            recorded: Recorded {
                complexity: a.complexity,
                crap: a.crap.map(|tenths| tenths as f64 / 10.0),
            },
        }
    }

    /// Whether the function is in the baseline and no worse than it was recorded
    /// in the scores it is over a limit in.
    pub(crate) fn allows(&self, risk: &Risk<'_>) -> bool {
        self.standing(risk) == Standing::Allowed
    }

    /// The baselined functions whose scores have moved from the recorded ones,
    /// degradations first and the biggest change first. It covers every baselined
    /// function still in the code, whether or not it is now over a limit, but not
    /// the ones failing the gate, which are reported as failures.
    ///
    /// Functions that share a name in one file share an entry, which holds the
    /// highest scores of those over a limit when it was recorded. So the highest
    /// scores are compared of those over a limit now; of a name with none over, of
    /// all of them. A shared-name function under the limits never moves the entry.
    /// Tests are left out unless the gate counts them.
    pub(crate) fn movements<'a>(
        &self,
        functions: &'a [FunctionMetrics],
        limits: &Thresholds,
        failing: &[Risk<'_>],
    ) -> Vec<Movement<'a>> {
        let over: Vec<&'a FunctionMetrics> = risky(functions, limits)
            .iter()
            .map(|r| r.function)
            .collect();
        let over_keys: HashSet<_> = over.iter().map(|f| key(f)).collect();
        let rest = functions
            .iter()
            .filter(|f| limits.include_tests || !f.is_test)
            .filter(|f| !over_keys.contains(&key(f)));

        let mut highest: BTreeMap<(String, String), (&'a FunctionMetrics, u32, Option<f64>)> =
            BTreeMap::new();
        for f in over.iter().copied().chain(rest) {
            let k = key(f);
            if !self.entries.contains_key(&k) {
                continue;
            }
            highest
                .entry(k)
                .and_modify(|(shown, complexity, crap)| {
                    if f.complexity > *complexity {
                        *shown = f;
                    }
                    *complexity = (*complexity).max(f.complexity);
                    *crap = match (*crap, f.crap) {
                        (Some(x), Some(y)) => Some(x.max(y)),
                        (x, y) => x.or(y),
                    };
                })
                .or_insert((f, f.complexity, f.crap));
        }
        let failing: HashSet<_> = failing.iter().map(|r| key(r.function)).collect();
        let mut moves: Vec<(Movement<'a>, (u32, u64))> = highest
            .into_iter()
            .filter(|(k, _)| !failing.contains(k))
            .filter_map(|(k, (function, complexity, crap))| {
                let a = self.entries.get(&k)?;
                let crap_tenths = crap.map(tenths);
                let crap_direction = match (crap_tenths, a.crap) {
                    (Some(now), Some(then)) => direction(now, then),
                    _ => None,
                };
                let complexity_direction = direction(complexity, a.complexity);
                if complexity_direction.is_none() && crap_direction.is_none() {
                    return None;
                }
                let movement = Movement {
                    function,
                    recorded: Recorded {
                        complexity: a.complexity,
                        crap: a.crap.map(|t| t as f64 / 10.0),
                    },
                    now: Recorded {
                        complexity,
                        crap: crap_tenths.map(|t| t as f64 / 10.0),
                    },
                    complexity: complexity_direction,
                    crap: crap_direction,
                };
                // How far it moved, for ordering, in the direction that puts it
                // in its group: a degraded function by how much got worse, not
                // netted against what got better.
                let group = if movement.degraded() {
                    Direction::Worse
                } else {
                    Direction::Better
                };
                let size = (
                    if complexity_direction == Some(group) {
                        complexity.abs_diff(a.complexity)
                    } else {
                        0
                    },
                    match (crap_direction == Some(group), crap_tenths, a.crap) {
                        (true, Some(now), Some(then)) => now.abs_diff(then),
                        _ => 0,
                    },
                );
                Some((movement, size))
            })
            .collect();
        moves.sort_by(|(a, size_a), (b, size_b)| {
            b.degraded()
                .cmp(&a.degraded())
                .then(size_b.cmp(size_a))
                .then_with(|| a.function.file.cmp(&b.function.file))
                .then_with(|| a.function.name.cmp(&b.function.name))
        });
        moves.into_iter().map(|(m, _)| m).collect()
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

    /// `f` judged as over both limits, so both scores are compared.
    fn both(f: &FunctionMetrics) -> Risk<'_> {
        over(f, true, true)
    }

    /// `f` judged as over the limits flagged.
    fn over(f: &FunctionMetrics, complexity: bool, crap: bool) -> Risk<'_> {
        Risk {
            function: f,
            complexity,
            crap,
        }
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
            b.allows(&both(&func("src/a.rs", "f", 15, Some(30.04)))),
            "as recorded"
        );
        assert!(
            b.allows(&both(&func("src/a.rs", "f", 9, Some(2.0)))),
            "better"
        );
        assert!(
            b.allows(&both(&func("src/a.rs", "f", 15, None))),
            "coverage now unknown"
        );
        assert!(
            !b.allows(&both(&func("src/a.rs", "f", 16, Some(30.0)))),
            "more complex"
        );
        assert!(
            !b.allows(&both(&func("src/a.rs", "f", 15, Some(31.0)))),
            "worse CRAP"
        );
    }

    #[test]
    fn a_function_recorded_and_read_back_is_never_failed_for_rounding() {
        for crap in [29.2, 39.2, 0.1, 0.3, 18.0, 41.26, 1e6 + 0.05] {
            let f = func("src/a.rs", "f", 10, Some(crap));
            let read = Baseline::decode(&baseline(std::slice::from_ref(&f)).encode()).expect("ok");
            assert!(read.allows(&both(&f)), "{crap}");
        }
    }

    #[test]
    fn standing_says_which_scores_are_worse_than_recorded() {
        let b = baseline(&[func("src/a.rs", "f", 29, Some(29.2))]);
        assert_eq!(
            b.standing(&both(&func("src/a.rs", "f", 29, Some(29.2)))),
            Standing::Allowed
        );
        assert_eq!(
            b.standing(&both(&func("src/a.rs", "f", 20, None))),
            Standing::Allowed
        );
        assert_eq!(
            b.standing(&both(&func("src/a.rs", "other", 1, None))),
            Standing::New
        );
        assert_eq!(
            b.standing(&both(&func("src/b.rs", "f", 1, None))),
            Standing::New,
            "other file"
        );

        let recorded = Recorded {
            complexity: 29,
            crap: Some(29.2),
        };
        assert_eq!(
            b.standing(&both(&func("src/a.rs", "f", 31, Some(29.2)))),
            Standing::Worse {
                complexity: true,
                crap: false,
                recorded
            }
        );
        assert_eq!(
            b.standing(&both(&func("src/a.rs", "f", 29, Some(40.0)))),
            Standing::Worse {
                complexity: false,
                crap: true,
                recorded
            }
        );
        assert_eq!(
            b.standing(&both(&func("src/a.rs", "f", 30, Some(40.0)))),
            Standing::Worse {
                complexity: true,
                crap: true,
                recorded
            }
        );
    }

    #[test]
    fn a_crap_that_was_never_recorded_is_worse_once_it_is_known() {
        let b = baseline(&[func("src/a.rs", "g", 7, None)]);
        assert_eq!(
            b.standing(&both(&func("src/a.rs", "g", 7, Some(9.0)))),
            Standing::Worse {
                complexity: false,
                crap: true,
                recorded: Recorded {
                    complexity: 7,
                    crap: None
                }
            }
        );
        assert_eq!(
            b.standing(&both(&func("src/a.rs", "g", 7, None))),
            Standing::Allowed
        );
    }

    #[test]
    fn allows_is_exactly_the_allowed_standing() {
        let b = baseline(&[
            func("src/a.rs", "f", 15, Some(30.04)),
            func("src/a.rs", "g", 7, None),
        ]);
        for probe in [
            func("src/a.rs", "f", 15, Some(30.04)),
            func("src/a.rs", "f", 16, Some(1.0)),
            func("src/a.rs", "f", 15, Some(99.0)),
            func("src/a.rs", "f", 15, None),
            func("src/a.rs", "g", 7, Some(1.0)),
            func("src/a.rs", "g", 7, None),
            func("src/a.rs", "nope", 1, None),
        ] {
            assert_eq!(
                b.allows(&both(&probe)),
                b.standing(&both(&probe)) == Standing::Allowed,
                "{probe:?}"
            );
        }
    }

    #[test]
    fn a_function_not_in_the_baseline_is_not_allowed() {
        let b = baseline(&[func("src/a.rs", "f", 15, Some(30.0))]);
        assert!(
            !b.allows(&both(&func("src/a.rs", "g", 1, Some(1.0)))),
            "other name"
        );
        assert!(
            !b.allows(&both(&func("src/b.rs", "f", 1, Some(1.0)))),
            "other file"
        );
    }

    #[test]
    fn an_entry_recorded_without_a_known_crap_does_not_allow_one_that_is_known_now() {
        let b = baseline(&[func("src/a.rs", "f", 15, None)]);
        assert!(
            !b.allows(&both(&func("src/a.rs", "f", 15, Some(500.0)))),
            "new CRAP"
        );
        assert!(
            b.allows(&both(&func("src/a.rs", "f", 15, None))),
            "still unknown"
        );
        assert!(
            !b.allows(&both(&func("src/a.rs", "f", 16, None))),
            "more complex"
        );
    }

    #[test]
    fn a_function_whose_coverage_is_unknown_now_is_judged_on_complexity_alone() {
        let b = baseline(&[func("src/a.rs", "f", 15, Some(30.0))]);
        assert!(b.allows(&both(&func("src/a.rs", "f", 15, None))));
        assert!(!b.allows(&both(&func("src/a.rs", "f", 16, None))));
    }

    #[test]
    fn a_function_over_only_on_complexity_is_judged_on_complexity() {
        // parse: complexity 29 is over the limit; CRAP 29.2 is under it.
        let b = baseline(&[func("src/a.rs", "parse", 29, Some(29.2))]);
        let drifted = func("src/a.rs", "parse", 29, Some(29.4));
        assert_eq!(
            b.standing(&over(&drifted, true, false)),
            Standing::Allowed,
            "CRAP moved but is not over its limit"
        );
        assert!(b.allows(&over(&drifted, true, false)));
        let more_complex = func("src/a.rs", "parse", 30, Some(29.4));
        assert_eq!(
            b.standing(&over(&more_complex, true, false)),
            Standing::Worse {
                complexity: true,
                crap: false,
                recorded: Recorded {
                    complexity: 29,
                    crap: Some(29.2)
                }
            },
            "only the score over its limit is named"
        );
    }

    #[test]
    fn a_function_over_only_on_crap_is_judged_on_crap() {
        // post_multipart: CRAP 30.0 is at the limit; complexity 5 is far under.
        let b = baseline(&[func("src/a.rs", "post", 5, Some(30.0))]);
        let more_complex = func("src/a.rs", "post", 6, Some(30.0));
        assert_eq!(
            b.standing(&over(&more_complex, false, true)),
            Standing::Allowed,
            "complexity moved but is not over its limit"
        );
        let worse = func("src/a.rs", "post", 5, Some(30.2));
        assert_eq!(
            b.standing(&over(&worse, false, true)),
            Standing::Worse {
                complexity: false,
                crap: true,
                recorded: Recorded {
                    complexity: 5,
                    crap: Some(30.0)
                }
            }
        );
    }

    #[test]
    fn crossing_a_limit_is_judged_against_the_recorded_value() {
        // CRAP 29.2 was recorded under a limit of 30; 30.2 is over it, and worse.
        let b = baseline(&[func("src/a.rs", "parse", 29, Some(29.2))]);
        let crossed = func("src/a.rs", "parse", 29, Some(30.2));
        assert_eq!(
            b.standing(&over(&crossed, true, true)),
            Standing::Worse {
                complexity: false,
                crap: true,
                recorded: Recorded {
                    complexity: 29,
                    crap: Some(29.2)
                }
            }
        );
        assert!(!b.allows(&over(&crossed, true, true)));
    }

    #[test]
    fn a_function_over_both_limits_is_judged_on_both() {
        let b = baseline(&[func("src/a.rs", "rewrite", 39, Some(39.2))]);
        let more_complex = func("src/a.rs", "rewrite", 40, Some(39.2));
        let less_covered = func("src/a.rs", "rewrite", 39, Some(39.5));
        let same = func("src/a.rs", "rewrite", 39, Some(39.2));
        assert!(!b.allows(&over(&more_complex, true, true)));
        assert!(!b.allows(&over(&less_covered, true, true)));
        assert!(b.allows(&over(&same, true, true)));
    }

    #[test]
    fn a_crap_never_recorded_is_only_held_against_a_function_over_the_crap_limit() {
        let b = baseline(&[func("src/a.rs", "g", 7, None)]);
        let known = func("src/a.rs", "g", 7, Some(9.0));
        assert!(
            b.allows(&over(&known, true, false)),
            "not over the CRAP limit"
        );
        assert!(
            !b.allows(&over(&known, false, true)),
            "over the CRAP limit, and none was accepted"
        );
    }

    /// The limits the tests judge against: complexity 15 and CRAP 30.
    fn limits(include_tests: bool) -> Thresholds {
        Thresholds {
            complexity: 15,
            crap: 30.0,
            include_tests,
        }
    }

    fn moved<'a>(b: &Baseline, functions: &'a [FunctionMetrics]) -> Vec<Movement<'a>> {
        b.movements(functions, &limits(false), &[])
    }

    #[test]
    fn a_baselined_function_that_moved_says_which_way_each_score_went() {
        let b = baseline(&[func("src/a.rs", "parse", 29, Some(29.2))]);

        let worse_crap = [func("src/a.rs", "parse", 29, Some(29.4))];
        let m = moved(&b, &worse_crap);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].complexity, None, "complexity is as recorded");
        assert_eq!(m[0].crap, Some(Direction::Worse));
        assert_eq!(m[0].recorded.complexity, 29);
        assert_eq!(m[0].recorded.crap, Some(29.2));
        assert_eq!(m[0].now.crap, Some(29.4));
        assert!(m[0].degraded() && !m[0].improved());

        let better = [func("src/a.rs", "parse", 20, Some(20.1))];
        let m = moved(&b, &better);
        assert_eq!(m[0].complexity, Some(Direction::Better));
        assert_eq!(m[0].crap, Some(Direction::Better));
        assert!(m[0].improved() && !m[0].degraded());

        let mixed = [func("src/a.rs", "parse", 31, Some(25.0))];
        let m = moved(&b, &mixed);
        assert_eq!(m[0].complexity, Some(Direction::Worse));
        assert_eq!(m[0].crap, Some(Direction::Better));
        assert!(
            m[0].degraded() && !m[0].improved(),
            "any worse score is a degradation"
        );
    }

    #[test]
    fn nothing_is_reported_for_a_function_as_recorded_or_not_in_the_baseline() {
        let b = baseline(&[func("src/a.rs", "parse", 29, Some(29.2))]);
        assert!(moved(&b, &[func("src/a.rs", "parse", 29, Some(29.2))]).is_empty());
        assert!(
            moved(&b, &[func("src/a.rs", "parse", 29, Some(29.15))]).is_empty(),
            "CRAP is compared as the baseline keeps it, in tenths rounded up"
        );
        assert!(moved(&b, &[func("src/a.rs", "other", 40, Some(90.0))]).is_empty());
        assert!(moved(&b, &[func("src/b.rs", "parse", 40, Some(90.0))]).is_empty());
        assert!(moved(&b, &[]).is_empty(), "the function is gone");
    }

    #[test]
    fn a_function_that_dropped_under_its_limits_still_reports_its_improvement() {
        // Movements look at every baselined function, not only those over a limit.
        let b = baseline(&[func("src/a.rs", "parse", 29, Some(29.2))]);
        let now = [func("src/a.rs", "parse", 3, Some(3.0))];
        let m = moved(&b, &now);
        assert_eq!(m.len(), 1);
        assert!(m[0].improved());
    }

    #[test]
    fn unknown_coverage_reports_no_crap_movement() {
        let b = baseline(&[
            func("src/a.rs", "f", 10, Some(12.0)),
            func("src/a.rs", "g", 10, None),
        ]);
        let now = [
            func("src/a.rs", "f", 10, None),
            func("src/a.rs", "g", 10, Some(11.0)),
        ];
        let m = moved(&b, &now);
        assert!(m.is_empty(), "no CRAP now, or none recorded: {m:?}");
    }

    #[test]
    fn same_name_functions_are_compared_by_their_highest_scores() {
        let b = baseline(&[
            func("src/a.rs", "new", 29, Some(29.2)),
            func("src/a.rs", "new", 5, Some(35.0)),
        ]);
        let now = [
            func("src/a.rs", "new", 30, Some(35.0)),
            func("src/a.rs", "new", 5, Some(10.0)),
        ];
        let m = moved(&b, &now);
        assert_eq!(m.len(), 1, "one entry, one movement");
        assert_eq!(m[0].complexity, Some(Direction::Worse));
        assert_eq!(m[0].crap, None, "35.0 is still the highest CRAP");
        assert_eq!(
            m[0].function.complexity, 30,
            "the most complex one is named"
        );
    }

    #[test]
    fn same_name_functions_over_a_limit_are_compared_by_their_highest_scores() {
        let b = baseline(&[
            func("src/a.rs", "new", 29, Some(29.2)),
            func("src/a.rs", "new", 20, Some(35.0)),
        ]);
        // Both are over a limit now: complexity 31 and CRAP 36.0 are the highest.
        let now = [
            func("src/a.rs", "new", 31, Some(29.2)),
            func("src/a.rs", "new", 20, Some(36.0)),
        ];
        let m = moved(&b, &now);
        assert_eq!(m.len(), 1, "one entry, one movement");
        assert_eq!(m[0].now.complexity, 31);
        assert_eq!(m[0].now.crap, Some(36.0));
        assert_eq!(m[0].complexity, Some(Direction::Worse));
        assert_eq!(m[0].crap, Some(Direction::Worse));
        assert_eq!(m[0].function.complexity, 31, "the most complex is named");
    }

    #[test]
    fn same_name_functions_all_under_the_limits_are_compared_by_their_highest_scores() {
        // None is over a limit now, so the entry is stale: its highest scores now
        // are what improved.
        let b = baseline(&[
            func("src/a.rs", "new", 29, Some(29.2)),
            func("src/a.rs", "new", 20, Some(35.0)),
        ]);
        let now = [
            func("src/a.rs", "new", 8, Some(9.0)),
            func("src/a.rs", "new", 12, Some(12.5)),
        ];
        let m = moved(&b, &now);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].now.complexity, 12);
        assert_eq!(m[0].now.crap, Some(12.5));
        assert!(m[0].improved());
    }

    #[test]
    fn a_crap_that_was_not_recorded_is_shown_as_such_beside_a_complexity_that_moved() {
        let b = baseline(&[func("src/a.rs", "g", 20, None)]);
        let now = [func("src/a.rs", "g", 21, Some(25.0))];
        let m = moved(&b, &now);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].crap, None, "no recorded CRAP to compare with");
        assert_eq!(m[0].complexity_text(), "20 → 21 (worse)");
        assert_eq!(m[0].crap_text(), "25.0, none recorded");
    }

    #[test]
    fn test_functions_are_left_out_unless_the_gate_counts_them() {
        let b = baseline(&[func("src/a.rs", "t", 20, Some(20.0))]);
        let mut test_fn = func("src/a.rs", "t", 25, Some(25.0));
        test_fn.is_test = true;
        let fns = [test_fn];
        assert!(b.movements(&fns, &limits(false), &[]).is_empty());
        assert_eq!(b.movements(&fns, &limits(true), &[]).len(), 1);
    }

    #[test]
    fn a_function_that_fails_the_gate_is_not_reported_again() {
        let b = baseline(&[func("src/a.rs", "parse", 29, Some(29.2))]);
        let now = func("src/a.rs", "parse", 31, Some(29.2));
        let failing = [over(&now, true, false)];
        assert!(
            b.movements(std::slice::from_ref(&now), &limits(false), &failing)
                .is_empty()
        );
    }

    #[test]
    fn movements_list_degradations_first_then_the_biggest_change() {
        let b = baseline(&[
            func("src/a.rs", "better_a_lot", 30, Some(30.0)),
            func("src/a.rs", "better_a_little", 30, Some(30.0)),
            func("src/a.rs", "worse_a_little", 30, Some(30.0)),
            func("src/a.rs", "worse_a_lot", 30, Some(30.0)),
        ]);
        let now = [
            func("src/a.rs", "better_a_lot", 10, Some(10.0)),
            func("src/a.rs", "better_a_little", 29, Some(29.0)),
            func("src/a.rs", "worse_a_little", 31, Some(31.0)),
            func("src/a.rs", "worse_a_lot", 35, Some(35.0)),
        ];
        let names: Vec<_> = moved(&b, &now)
            .iter()
            .map(|m| m.function.name.clone())
            .collect();
        assert_eq!(
            names,
            [
                "worse_a_lot",
                "worse_a_little",
                "better_a_lot",
                "better_a_little"
            ]
        );
    }

    #[test]
    fn a_same_name_function_that_would_not_be_recorded_does_not_move_the_entry() {
        // Only the functions over a limit are recorded: A (29, 10.0). B is under
        // both limits, so a CRAP of 25.0 on it is not a movement of the entry.
        let a = func("src/a.rs", "new", 29, Some(10.0));
        let b_fn = func("src/a.rs", "new", 5, Some(25.0));
        let b = baseline(std::slice::from_ref(&a));
        let now = [a, b_fn];
        assert!(moved(&b, &now).is_empty(), "{:?}", moved(&b, &now));
    }

    #[test]
    fn the_crap_shown_is_the_one_compared() {
        // Compared in tenths rounded up, as the baseline keeps it, so 29.21 is
        // 29.3 and worse than 29.2; it must not be shown as 29.2.
        let b = baseline(&[func("src/a.rs", "parse", 29, Some(29.2))]);
        let now = [func("src/a.rs", "parse", 29, Some(29.21))];
        let m = moved(&b, &now);
        assert_eq!(m[0].crap, Some(Direction::Worse));
        assert_eq!(m[0].now.crap, Some(29.3));
    }

    #[test]
    fn a_small_degradation_ranks_by_what_got_worse_not_by_what_got_better() {
        // `mixed` is one more complex but 50 less CRAP; `plain` is five more
        // complex. Both are degraded; the larger degradation comes first.
        let b = baseline(&[
            func("src/a.rs", "mixed", 20, Some(80.0)),
            func("src/a.rs", "plain", 20, Some(20.0)),
        ]);
        let now = [
            func("src/a.rs", "mixed", 21, Some(30.0)),
            func("src/a.rs", "plain", 25, Some(25.0)),
        ];
        let names: Vec<_> = moved(&b, &now)
            .iter()
            .map(|m| m.function.name.clone())
            .collect();
        assert_eq!(names, ["plain", "mixed"]);
    }

    #[test]
    fn what_got_better_does_not_count_towards_how_much_a_function_degraded() {
        // Both are one more complex. `offset` also has 50 less CRAP; `plain` has a
        // little more. Netting would put `offset` first; only what got worse counts.
        let b = baseline(&[
            func("src/a.rs", "offset", 20, Some(80.0)),
            func("src/a.rs", "plain", 20, Some(20.0)),
        ]);
        let now = [
            func("src/a.rs", "offset", 21, Some(30.0)),
            func("src/a.rs", "plain", 21, Some(20.1)),
        ];
        let names: Vec<_> = moved(&b, &now)
            .iter()
            .map(|m| m.function.name.clone())
            .collect();
        assert_eq!(names, ["plain", "offset"]);

        // And a complexity that got better does not count for a function
        // degraded only in its CRAP: 10 fewer complexity, 0.2 more CRAP, against
        // one more complexity.
        let b = baseline(&[
            func("src/a.rs", "crap_only", 30, Some(30.0)),
            func("src/a.rs", "complexity_up", 20, Some(20.0)),
        ]);
        let now = [
            func("src/a.rs", "crap_only", 20, Some(30.2)),
            func("src/a.rs", "complexity_up", 21, Some(21.0)),
        ];
        let names: Vec<_> = moved(&b, &now)
            .iter()
            .map(|m| m.function.name.clone())
            .collect();
        assert_eq!(names, ["complexity_up", "crap_only"]);
    }

    #[test]
    fn path_separators_do_not_matter() {
        let b = baseline(&[func("crates\\x\\src\\a.rs", "f", 15, Some(30.0))]);
        assert!(b.allows(&both(&func("crates/x/src/a.rs", "f", 15, Some(30.0)))));
        let recorded = baseline(&[func("crates/x/src/a.rs", "f", 15, Some(30.0))]);
        assert_eq!(b.encode(), recorded.encode());
    }

    #[test]
    fn functions_sharing_a_name_share_their_highest_scores() {
        let b = baseline(&[
            func("src/a.rs", "new", 12, Some(20.0)),
            func("src/a.rs", "new", 18, None),
            func("src/a.rs", "new", 5, Some(33.0)),
        ]);
        assert_eq!(b.len(), 1);
        assert!(b.allows(&both(&func("src/a.rs", "new", 18, Some(33.0)))));
        assert!(!b.allows(&both(&func("src/a.rs", "new", 19, Some(1.0)))));
    }

    #[test]
    fn closures_are_recorded_under_one_name_per_file() {
        let b = baseline(&[func("src/a.rs", "", 11, Some(12.0))]);
        assert!(b.allows(&both(&func("src/a.rs", "", 11, Some(12.0)))));
        assert!(b.allows(&both(&func("src/a.rs", "<closure>", 11, Some(12.0)))));
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
