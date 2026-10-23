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
//! The file is plain text, one tab-separated `file`, `function`, `complexity`,
//! `CRAP` line per function (`-` when its coverage is unknown), with `#` comment
//! lines.

use std::{collections::BTreeMap, fmt::Write as _};

use anyhow::{Context, Result, bail};

use crate::complexity::metrics::FunctionMetrics;

const HEADER: &str = "\
# jci-coverage complexity baseline: functions allowed to stay over a maximum.
# Regenerate with `jci-coverage complexity --write-baseline <PATH>`; do not edit.
# file<TAB>function<TAB>complexity<TAB>CRAP
";

/// The name closures are recorded under.
const CLOSURE: &str = "<closure>";

/// The scores a baselined function may not exceed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Allowance {
    complexity: u32,
    /// `None` when its coverage was unknown, which allows any CRAP.
    crap: Option<f64>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Baseline {
    entries: BTreeMap<(String, String), Allowance>,
}

fn key(f: &FunctionMetrics) -> (String, String) {
    let name = if f.name.is_empty() { CLOSURE } else { &f.name };
    (f.file.clone(), name.to_string())
}

/// CRAP rounded up to the one decimal the file keeps, so a function is never
/// failed for the digits that were cut off.
fn round_up(crap: f64) -> f64 {
    (crap * 10.0).ceil() / 10.0
}

impl Baseline {
    /// A baseline allowing exactly these functions as they are.
    pub(crate) fn from_functions<'a>(
        functions: impl IntoIterator<Item = &'a FunctionMetrics>,
    ) -> Self {
        let mut entries: BTreeMap<(String, String), Allowance> = BTreeMap::new();
        for f in functions {
            let crap = f.crap.map(round_up);
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
        Self { entries }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn render(&self) -> String {
        let mut out = HEADER.to_string();
        for ((file, name), a) in &self.entries {
            let crap = a
                .crap
                .map_or_else(|| "-".to_string(), |c| format!("{c:.1}"));
            let _ = writeln!(out, "{file}\t{name}\t{}\t{crap}", a.complexity);
        }
        out
    }

    pub(crate) fn parse(text: &str) -> Result<Self> {
        let mut entries = BTreeMap::new();
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let at = || format!("baseline line {}", i + 1);
            let cols: Vec<&str> = line.split('\t').collect();
            let [file, name, complexity, crap] = cols[..] else {
                bail!(
                    "{}: expected 4 tab-separated columns, found {}",
                    at(),
                    cols.len()
                );
            };
            let complexity = complexity
                .parse()
                .with_context(|| format!("{}: complexity {complexity:?}", at()))?;
            let crap = match crap {
                "-" => None,
                c => Some(
                    c.parse::<f64>()
                        .ok()
                        .filter(|c| c.is_finite() && *c >= 0.0)
                        .with_context(|| format!("{}: CRAP {c:?}", at()))?,
                ),
            };
            entries.insert(
                (file.to_string(), name.to_string()),
                Allowance { complexity, crap },
            );
        }
        Ok(Self { entries })
    }

    /// Whether `f` is in the baseline and no worse than it was recorded.
    pub(crate) fn allows(&self, f: &FunctionMetrics) -> bool {
        let Some(a) = self.entries.get(&key(f)) else {
            return false;
        };
        f.complexity <= a.complexity
            && match (f.crap, a.crap) {
                (Some(now), Some(then)) => now <= then + 1e-9,
                _ => true,
            }
    }

    /// How many entries match none of the functions currently over a maximum:
    /// the function is gone or has dropped under it.
    pub(crate) fn unused<'a>(&self, over: impl IntoIterator<Item = &'a FunctionMetrics>) -> usize {
        let used: std::collections::HashSet<_> = over.into_iter().map(key).collect();
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
        Baseline::from_functions(functions)
    }

    #[test]
    fn a_baseline_round_trips_through_its_file() {
        let b = baseline(&[
            func("src/b.rs", "zed", 20, Some(41.26)),
            func("src/a.rs", "alpha", 12, None),
        ]);
        let text = b.render();
        assert!(
            text.starts_with("# jci-coverage complexity baseline"),
            "{text}"
        );
        let rows: Vec<_> = text.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            rows,
            ["src/a.rs\talpha\t12\t-", "src/b.rs\tzed\t20\t41.3"],
            "sorted, CRAP rounded up"
        );
        assert_eq!(Baseline::parse(&text).expect("parses"), b);
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn parse_skips_comments_and_blank_lines_and_rejects_bad_rows() {
        let ok = Baseline::parse("# c\n\nsrc/a.rs\tf\t3\t4.5\n").expect("parses");
        assert_eq!(ok.len(), 1);

        for (text, wanted) in [
            ("src/a.rs\tf\t3\n", "line 1: expected 4"),
            ("# c\nsrc/a.rs\tf\tx\t1.0\n", "line 2: complexity"),
            ("src/a.rs\tf\t3\tNaN\n", "CRAP"),
            ("src/a.rs\tf\t3\t-2\n", "CRAP"),
            ("src/a.rs\tf\t3\tabc\n", "CRAP"),
        ] {
            let err = format!("{:#}", Baseline::parse(text).unwrap_err());
            assert!(err.contains(wanted), "{text:?}: {err}");
        }
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
        assert!(b.render().contains("src/a.rs\t<closure>\t11\t12.0"));
        assert!(b.allows(&func("src/a.rs", "", 11, Some(12.0))));
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
