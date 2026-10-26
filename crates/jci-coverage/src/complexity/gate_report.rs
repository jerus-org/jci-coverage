//! The report printed when the gate fails.
//!
//! A bare list of numbers says that a function is over a limit but not why, nor
//! what to do. This lays the failures out as a table, one row per function, with
//! the cause (complexity, CRAP, or both), the coverage behind the CRAP score and,
//! for CRAP, how much more of the function must be covered to pass, or that no
//! amount of testing can. It ends with the ways to resolve a failure.
//!
//! CRAP is `complexity² × (1 − coverage)³ + complexity`, which is never below
//! the complexity: a function whose complexity is at or over the CRAP limit
//! cannot be fixed by tests, only by making it simpler.

use crate::complexity::{
    baseline::Baseline,
    metrics::{FunctionMetrics, Risk, crap},
};

/// The limits the gate applied. `0` is a check that was off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Limits {
    pub(crate) complexity: u32,
    pub(crate) crap: f64,
}

/// What it would take to bring a function's CRAP under the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrapFix {
    /// Cover this many more of its own lines (`covered` of `lines` in all).
    MoreCoverage { more: usize, covered: usize },
    /// Even fully covered its CRAP would be its complexity, which is at or over
    /// the limit, so it has to become simpler.
    ReduceComplexity,
}

/// What it takes to bring `f`'s CRAP under `max_crap`. `f` must have a CRAP score.
pub(crate) fn crap_fix(f: &FunctionMetrics, max_crap: f64) -> CrapFix {
    for covered in f.covered + 1..=f.lines {
        if crap(f.complexity, covered, f.lines) < max_crap {
            return CrapFix::MoreCoverage {
                more: covered - f.covered,
                covered,
            };
        }
    }
    CrapFix::ReduceComplexity
}

/// The "Fix" cell: what would bring `r` back under the limits.
fn fix(r: &Risk<'_>, limits: &Limits) -> String {
    let f = r.function;
    if r.complexity {
        return format!("reduce complexity below {}", limits.complexity);
    }
    match crap_fix(f, limits.crap) {
        CrapFix::MoreCoverage { more, covered } => format!(
            "cover {} more {} ({covered} of {})",
            more,
            if more == 1 { "line" } else { "lines" },
            f.lines
        ),
        CrapFix::ReduceComplexity => format!(
            "tests alone cannot help: fully covered, CRAP would still be {} (limit {}); reduce complexity",
            f.complexity, limits.crap
        ),
    }
}

/// The "Baseline" cell: how a failing function stands against the baseline.
fn against_baseline(f: &FunctionMetrics, baseline: &Baseline) -> String {
    match baseline.recorded(f) {
        None => "not in the baseline".to_string(),
        Some(rec) => {
            let crap = rec
                .crap
                .map_or_else(String::new, |c| format!(", CRAP {c:.1}"));
            format!(
                "worse than the baseline (recorded: complexity {}{crap})",
                rec.complexity
            )
        }
    }
}

/// The table: a header, a rule and a row for each cell vector, padded to the
/// widest cell of each column.
fn table(header: &[String], rows: &[Vec<String>]) -> String {
    let widths: Vec<usize> = (0..header.len())
        .map(|c| {
            std::iter::once(header[c].chars().count())
                .chain(rows.iter().map(|r| r[c].chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[String]| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, w)| format!("{cell:<w$}"))
            .collect();
        format!("| {} |", padded.join(" | "))
    };
    let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w + 2)).collect();
    let mut out = line(header);
    out += &format!("\n|{}|", rule.join("|"));
    for row in rows {
        out += &format!("\n{}", line(row));
    }
    out
}

/// The "How to resolve" section: only the options that apply to the limits set.
fn resolution(limits: &Limits, with_baseline: bool) -> String {
    let (complexity_on, crap_on) = (limits.complexity > 0, limits.crap > 0.0);
    let flags = [
        complexity_on.then(|| format!("--max-complexity {}", limits.complexity)),
        crap_on.then(|| format!("--max-crap {}", limits.crap)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let names = [
        complexity_on.then_some("max_complexity"),
        crap_on.then_some("max_crap"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" and ");
    let options = [
        complexity_on.then_some("--max-complexity N"),
        crap_on.then_some("--max-crap N"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" and ");

    let mut items = vec![
        "Reduce its complexity: split it into smaller functions. This is what a\n     function marked \"complexity\" needs, and any whose Fix column says that\n     tests alone cannot help."
            .to_string(),
    ];
    if crap_on {
        items.push(
            "Add tests for its own lines. CRAP falls as coverage rises; the Fix\n     column says how many covered lines are enough."
                .to_string(),
        );
    }
    let regenerate = if with_baseline {
        " Regenerate it the same way if the change is intended."
    } else {
        ""
    };
    items.push(format!(
        "Accept it for now, recording today's state so that only new or worse\n     functions fail:\n         jci-coverage complexity {flags} --write-baseline\n     then commit {DEFAULT} and pass it with --baseline\n     (orb job parameter baseline: {DEFAULT}).{regenerate}",
        DEFAULT = crate::complexity::DEFAULT_BASELINE
    ));
    items.push(format!(
        "Change the limits if they do not suit this project:\n     {options}\n     (orb job parameters {names}); 0 turns a check off."
    ));

    let mut out = "How to resolve a failure, for each function above:".to_string();
    for (i, item) in items.iter().enumerate() {
        out += &format!("\n  {}. {item}", i + 1);
    }
    out
}

/// The report for the functions that fail the gate, worst first.
pub(crate) fn render(failing: &[Risk<'_>], limits: &Limits, baseline: Option<&Baseline>) -> String {
    let (complexity_on, crap_on) = (limits.complexity > 0, limits.crap > 0.0);

    let by_complexity = failing.iter().filter(|r| r.complexity).count();
    let by_crap = failing.iter().filter(|r| r.crap).count();
    let tests_fix = failing
        .iter()
        .filter(|r| r.crap && !r.complexity)
        .filter(|r| {
            matches!(
                crap_fix(r.function, limits.crap),
                CrapFix::MoreCoverage { .. }
            )
        })
        .count();
    let mut causes = Vec::new();
    if complexity_on {
        causes.push(format!(
            "{by_complexity} over the complexity limit ({})",
            limits.complexity
        ));
    }
    if crap_on {
        causes.push(format!("{by_crap} over the CRAP limit ({})", limits.crap));
    }
    let mut summary = format!(
        "{} function(s) fail the gate: {}",
        failing.len(),
        causes.join(", ")
    );
    if crap_on && by_crap > 0 {
        summary += &format!("; {tests_fix} of the CRAP failures can be fixed with tests alone");
    }
    summary.push('.');

    let mut header: Vec<String> = ["Function", "Location"].map(String::from).to_vec();
    header.push(if complexity_on {
        format!("Complexity (max {})", limits.complexity)
    } else {
        "Complexity".to_string()
    });
    header.push("Covered".to_string());
    header.push(if crap_on {
        format!("CRAP (max {})", limits.crap)
    } else {
        "CRAP".to_string()
    });
    header.push("Over".to_string());
    if baseline.is_some() {
        header.push("Baseline".to_string());
    }
    header.push("Fix".to_string());

    let rows: Vec<Vec<String>> = failing
        .iter()
        .map(|r| {
            let f = r.function;
            let covered = if f.lines == 0 {
                "n/a".to_string()
            } else {
                let pct = (f.covered as f64 * 100.0 / f.lines as f64).round();
                format!("{}/{} ({pct}%)", f.covered, f.lines)
            };
            let mut over: Vec<&str> = Vec::new();
            if r.complexity {
                over.push("complexity");
            }
            if r.crap {
                // Say where the CRAP comes from: missing coverage, or a
                // complexity that no amount of testing gets under the limit.
                over.push(match crap_fix(f, limits.crap) {
                    CrapFix::MoreCoverage { .. } => "CRAP (coverage)",
                    CrapFix::ReduceComplexity => "CRAP (complexity)",
                });
            }
            let mut row = vec![
                if f.name.is_empty() {
                    "<closure>".to_string()
                } else {
                    f.name.clone()
                },
                format!("{}:{}", f.file, f.line),
                f.complexity.to_string(),
                covered,
                f.crap
                    .map_or_else(|| "n/a".to_string(), |c| format!("{c:.1}")),
                over.join(", "),
            ];
            if let Some(b) = baseline {
                row.push(against_baseline(f, b));
            }
            row.push(fix(r, limits));
            row
        })
        .collect();

    format!(
        "{summary}\n\n{}\n\n{}",
        table(&header, &rows),
        resolution(limits, baseline.is_some())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn func(
        name: &str,
        complexity: u32,
        lines: usize,
        covered: usize,
        crap_score: Option<f64>,
    ) -> FunctionMetrics {
        FunctionMetrics {
            file: "src/a.rs".into(),
            name: name.into(),
            line: 10,
            complexity,
            lines,
            covered,
            crap: crap_score,
            is_test: false,
        }
    }

    /// A function with the CRAP score its coverage implies.
    fn scored(name: &str, complexity: u32, lines: usize, covered: usize) -> FunctionMetrics {
        func(
            name,
            complexity,
            lines,
            covered,
            (lines > 0).then(|| crap(complexity, covered, lines)),
        )
    }

    fn risk(function: &FunctionMetrics, complexity: bool, crap: bool) -> Risk<'_> {
        Risk {
            function,
            complexity,
            crap,
        }
    }

    const LIMITS: Limits = Limits {
        complexity: 15,
        crap: 30.0,
    };

    #[test]
    fn tests_can_fix_a_function_whose_crap_comes_from_missing_coverage() {
        // 5 × 5 × 1 + 5 = 30 uncovered; one covered line of 26 is enough.
        let f = scored("post", 5, 26, 0);
        assert_eq!(
            crap_fix(&f, 30.0),
            CrapFix::MoreCoverage {
                more: 1,
                covered: 1
            }
        );
        // 10 × 10 × (1 − k/20)³ + 10 < 30 needs 20 × (1 − (20/100)^(1/3)) ≈ 8.3,
        // so 9 of 20 lines.
        let g = scored("g", 10, 20, 0);
        assert_eq!(
            crap_fix(&g, 30.0),
            CrapFix::MoreCoverage {
                more: 9,
                covered: 9
            }
        );
    }

    #[test]
    fn only_the_missing_coverage_is_asked_for() {
        let f = scored("g", 10, 20, 5);
        assert_eq!(
            crap_fix(&f, 30.0),
            CrapFix::MoreCoverage {
                more: 4,
                covered: 9
            }
        );
    }

    #[test]
    fn tests_cannot_fix_a_function_whose_complexity_is_at_or_over_the_crap_limit() {
        let at = scored("at", 30, 10, 10);
        assert_eq!(crap_fix(&at, 30.0), CrapFix::ReduceComplexity);
        let over = scored("over", 39, 75, 71);
        assert_eq!(crap_fix(&over, 30.0), CrapFix::ReduceComplexity);
    }

    #[test]
    fn the_report_names_every_failing_function_with_no_cap() {
        let all: Vec<FunctionMetrics> = (0..12)
            .map(|n| scored(&format!("f{n:02}"), 16 + n, 10, 10))
            .collect();
        let risks: Vec<Risk<'_>> = all.iter().map(|f| risk(f, true, false)).collect();
        let out = render(&risks, &LIMITS, None);
        for f in &all {
            assert!(out.contains(&f.name), "{} missing:\n{out}", f.name);
        }
        assert!(!out.contains("more"), "nothing is elided:\n{out}");
        assert!(out.contains("12 function(s)"), "{out}");
    }

    #[test]
    fn the_report_is_a_table_with_the_cause_and_the_scores() {
        let a = scored("rewrite", 39, 75, 71);
        let b = scored("post_multipart", 5, 26, 0);
        let risks = [risk(&a, true, true), risk(&b, false, true)];
        let out = render(&risks, &LIMITS, None);
        let rows: Vec<&str> = out.lines().filter(|l| l.starts_with('|')).collect();
        assert_eq!(rows.len(), 4, "header, rule and two rows:\n{out}");
        assert!(
            rows[0].contains("Function") && rows[0].contains("Location"),
            "{}",
            rows[0]
        );
        assert!(rows[0].contains("Complexity (max 15)") && rows[0].contains("CRAP (max 30)"));
        assert!(
            rows[1].chars().all(|c| matches!(c, '|' | '-' | ' ')),
            "{}",
            rows[1]
        );
        assert!(
            rows[2].contains("rewrite")
                && rows[2].contains("src/a.rs:10")
                && rows[2].contains("39")
                && rows[2].contains("71/75 (95%)")
                && rows[2].contains("complexity, CRAP (complexity)"),
            "{}",
            rows[2]
        );
        assert!(
            rows[3].contains("post_multipart")
                && rows[3].contains("0/26 (0%)")
                && rows[3].contains("30.0")
                && rows[3].contains("CRAP (coverage)"),
            "{}",
            rows[3]
        );
        let widths: Vec<usize> = rows.iter().map(|r| r.chars().count()).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "aligned columns: {widths:?}"
        );
    }

    #[test]
    fn each_row_says_what_would_resolve_it() {
        let complex = scored("complex", 20, 10, 10);
        let untested = scored("untested", 5, 26, 0);
        let hopeless = scored("hopeless", 39, 75, 71);
        let risks = [
            risk(&complex, true, false),
            risk(&untested, false, true),
            risk(&hopeless, true, true),
        ];
        let out = render(&risks, &LIMITS, None);
        let row = |name: &str| {
            out.lines()
                .find(|l| l.starts_with('|') && l.contains(name))
                .unwrap_or_else(|| panic!("no row for {name}:\n{out}"))
                .to_string()
        };
        assert!(
            row("complex").contains("reduce complexity below 15"),
            "{}",
            row("complex")
        );
        assert!(
            row("untested").contains("cover 1 more line (1 of 26)"),
            "{}",
            row("untested")
        );
        assert!(
            row("hopeless").contains("reduce complexity below 15"),
            "{}",
            row("hopeless")
        );
    }

    #[test]
    fn a_crap_failure_that_tests_cannot_fix_says_why() {
        let f = scored("big", 40, 50, 50);
        let out = render(
            &[risk(&f, false, true)],
            &Limits {
                complexity: 0,
                crap: 30.0,
            },
            None,
        );
        assert!(out.contains("tests alone cannot"), "{out}");
        assert!(out.contains("fully covered"), "{out}");
        assert!(out.contains("CRAP would still be 40"), "{out}");
    }

    #[test]
    fn the_summary_counts_each_cause_and_what_tests_can_fix() {
        let a = scored("a", 20, 10, 10);
        let b = scored("b", 5, 26, 0);
        let c = scored("c", 39, 75, 71);
        let risks = [
            risk(&a, true, false),
            risk(&b, false, true),
            risk(&c, true, true),
        ];
        let out = render(&risks, &LIMITS, None);
        assert!(
            out.contains(
                "3 function(s) fail the gate: 2 over the complexity limit (15), 2 over the CRAP limit (30)"
            ),
            "{out}"
        );
        assert!(
            out.contains("1 of the CRAP failures can be fixed with tests alone"),
            "{out}"
        );
    }

    #[test]
    fn a_limit_that_was_off_has_no_column_text() {
        let f = scored("f", 20, 10, 10);
        let out = render(
            &[risk(&f, true, false)],
            &Limits {
                complexity: 15,
                crap: 0.0,
            },
            None,
        );
        assert!(!out.contains("CRAP (max"), "{out}");
        assert!(!out.contains("over the CRAP limit"), "{out}");
        assert!(out.contains("over the complexity limit (15)"), "{out}");
    }

    #[test]
    fn a_function_with_no_coverage_data_shows_n_a() {
        let f = func("bare", 20, 0, 0, None);
        let out = render(&[risk(&f, true, false)], &LIMITS, None);
        let row = out.lines().find(|l| l.contains("bare")).expect("row");
        assert!(row.contains("n/a"), "{row}");
    }

    #[test]
    fn a_closure_is_named_as_one() {
        let f = scored("", 20, 10, 10);
        let out = render(&[risk(&f, true, false)], &LIMITS, None);
        assert!(out.contains("<closure>"), "{out}");
    }

    #[test]
    fn the_report_ends_with_the_ways_to_resolve_a_failure() {
        let f = scored("f", 20, 10, 10);
        let out = render(&[risk(&f, true, false)], &LIMITS, None);
        let tail = out
            .split("How to resolve")
            .nth(1)
            .expect("a resolution section");
        assert!(tail.contains("Reduce"), "{tail}");
        assert!(tail.contains("Add tests"), "{tail}");
        assert!(tail.contains("--write-baseline"), "{tail}");
        assert!(tail.contains(".jci-coverage-baseline"), "{tail}");
        assert!(
            tail.contains("--max-complexity 15") && tail.contains("--max-crap 30"),
            "{tail}"
        );
        assert!(
            tail.contains("max_complexity") && tail.contains("max_crap"),
            "orb names: {tail}"
        );
    }

    #[test]
    fn the_resolution_text_only_offers_what_applies() {
        let f = scored("f", 20, 10, 10);
        let out = render(
            &[risk(&f, true, false)],
            &Limits {
                complexity: 15,
                crap: 0.0,
            },
            None,
        );
        let tail = out.split("How to resolve").nth(1).expect("section");
        assert!(
            !tail.contains("Add tests"),
            "complexity-only: tests do not help\n{tail}"
        );
        assert!(!tail.contains("--max-crap"), "{tail}");
    }

    #[test]
    fn a_function_worse_than_its_baseline_says_what_was_recorded() {
        let before = scored("rewrite", 29, 40, 40);
        let now = scored("rewrite", 31, 40, 40);
        let baseline = Baseline::from_functions([&before], crate::complexity::counter::Rules::Rca);
        let out = render(&[risk(&now, true, false)], &LIMITS, Some(&baseline));
        assert!(out.contains("worse than the baseline"), "{out}");
        assert!(out.contains("recorded: complexity 29"), "{out}");
        let tail = out.split("How to resolve").nth(1).expect("section");
        assert!(
            tail.contains("Regenerate"),
            "with a baseline in use: {tail}"
        );
    }

    #[test]
    fn a_function_not_in_the_baseline_is_new() {
        let other = scored("other", 29, 40, 40);
        let new = scored("fresh", 31, 40, 40);
        let baseline = Baseline::from_functions([&other], crate::complexity::counter::Rules::Rca);
        let out = render(&[risk(&new, true, false)], &LIMITS, Some(&baseline));
        assert!(out.contains("not in the baseline"), "{out}");
    }
}
