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

mod summary;

#[cfg(test)]
pub(crate) use summary::MARKER;
pub(crate) use summary::{MAX_ROWS, failing as failing_summary, passing as passing_summary};

use crate::complexity::{
    baseline::{Baseline, Standing},
    metrics::{FunctionMetrics, Risk, Thresholds, crap},
};

/// How to record the current state as a baseline: the command that does it, and
/// the file it writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) command: String,
    pub(crate) path: String,
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

/// A failing function, with what is worked out about its CRAP once.
struct Failure<'a> {
    function: &'a FunctionMetrics,
    complexity: bool,
    crap: bool,
    /// What fixes its CRAP; `Some` exactly when it fails the CRAP limit.
    fix: Option<CrapFix>,
}

impl<'a> Failure<'a> {
    fn new(risk: &Risk<'a>, limits: &Thresholds) -> Self {
        Self {
            function: risk.function,
            complexity: risk.complexity,
            crap: risk.crap,
            fix: risk.crap.then(|| crap_fix(risk.function, limits.crap)),
        }
    }
}

fn more_lines(more: usize, covered: usize, lines: usize) -> String {
    format!(
        "{more} more {} ({covered} of {lines})",
        if more == 1 { "line" } else { "lines" }
    )
}

/// What a function that is over both limits needs from tests once it has been
/// made simple enough: its complexity just under both limits, its coverage as
/// it is now. `None` when its CRAP would already pass.
fn coverage_after_reducing(f: &FunctionMetrics, limits: &Thresholds) -> Option<(usize, usize)> {
    let under_crap = (limits.crap.ceil() as u32).saturating_sub(1);
    let target = if limits.complexity > 0 {
        under_crap.min(limits.complexity - 1)
    } else {
        under_crap
    };
    if f.lines == 0 || crap(target, f.covered, f.lines) < limits.crap {
        return None;
    }
    let reduced = FunctionMetrics {
        complexity: target,
        ..f.clone()
    };
    match crap_fix(&reduced, limits.crap) {
        CrapFix::MoreCoverage { more, covered } => Some((more, covered)),
        CrapFix::ReduceComplexity => None,
    }
}

/// The "Fix" cell: what would bring a failing function back under the limits.
fn fix(r: &Failure<'_>, limits: &Thresholds) -> String {
    let f = r.function;
    if r.complexity {
        let mut text = format!("reduce complexity below {}", limits.complexity);
        // Simplifying lowers CRAP but may not be enough if it is also untested.
        if r.crap
            && let Some((more, covered)) = coverage_after_reducing(f, limits)
        {
            text += &format!(", then cover {}", more_lines(more, covered, f.lines));
        }
        return text;
    }
    match r.fix {
        Some(CrapFix::MoreCoverage { more, covered }) => {
            format!("cover {}", more_lines(more, covered, f.lines))
        }
        Some(CrapFix::ReduceComplexity) => format!(
            "tests alone cannot help: fully covered, CRAP would still be {} (limit {}); reduce complexity",
            f.complexity, limits.crap
        ),
        None => String::new(),
    }
}

/// The "Covered" cell. Rounded down, so only a fully covered function reads 100%.
fn covered_cell(f: &FunctionMetrics) -> String {
    if f.lines == 0 {
        return "n/a".to_string();
    }
    let percent = match f.covered * 100 / f.lines {
        0 if f.covered > 0 => "<1%".to_string(),
        p => format!("{p}%"),
    };
    format!("{}/{} ({percent})", f.covered, f.lines)
}

/// The "Baseline" cell: how a failing function stands against the baseline, and
/// in which score it is worse.
fn against_baseline(f: &FunctionMetrics, baseline: &Baseline) -> String {
    match baseline.standing(f) {
        Standing::New => "not in the baseline".to_string(),
        Standing::Allowed => "allowed by the baseline".to_string(),
        Standing::Worse {
            complexity,
            crap,
            recorded,
        } => {
            let mut parts = Vec::new();
            if complexity {
                parts.push(format!(
                    "complexity {} > {}",
                    f.complexity, recorded.complexity
                ));
            }
            if crap {
                match (f.crap, recorded.crap) {
                    (Some(now), Some(then)) => parts.push(format!("CRAP {now:.1} > {then:.1}")),
                    (Some(now), None) => parts.push(format!("CRAP {now:.1}, none recorded")),
                    (None, _) => {}
                }
            }
            format!("worse than the baseline: {}", parts.join("; "))
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
fn resolution(limits: &Thresholds, with_baseline: bool, record: &Record) -> String {
    let (complexity_on, crap_on) = (limits.complexity > 0, limits.crap > 0.0);
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
        "Reduce its complexity: split it into smaller functions. This is what a\n     function marked \"complexity\" needs, and any whose Fix column says\n     that tests alone cannot help."
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
    let pass = if record.path == crate::complexity::DEFAULT_BASELINE {
        String::new()
    } else {
        format!(" {}", record.path)
    };
    items.push(format!(
        "Accept it for now, recording today's state so that only new or worse\n     functions fail. Run this from the repository root, against a Cobertura\n     report made as in CI (jci-coverage report --format cobertura):\n         {}\n     then commit {} and pass it with --baseline{pass}\n     (orb job parameter baseline: {}).{regenerate}",
        record.command, record.path, record.path
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

/// The headline sentence: how many functions fail the gate, by cause, and how
/// many of the CRAP failures tests alone can fix.
fn headline(failures: &[Failure<'_>], limits: &Thresholds) -> String {
    let (complexity_on, crap_on) = (limits.complexity > 0, limits.crap > 0.0);
    let by_complexity = failures.iter().filter(|r| r.complexity).count();
    let by_crap = failures.iter().filter(|r| r.crap).count();
    let tests_fix = failures
        .iter()
        .filter(|r| r.crap && !r.complexity)
        .filter(|r| matches!(r.fix, Some(CrapFix::MoreCoverage { .. })))
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
        failures.len(),
        causes.join(", ")
    );
    if crap_on && by_crap > 0 {
        summary += &format!("; {tests_fix} of the CRAP failures can be fixed with tests alone");
    }
    summary.push('.');
    summary
}

/// The "Over" cell: which limits a function breaks. A CRAP failure says where
/// the CRAP comes from: missing coverage, or a complexity that no amount of
/// testing gets under the limit.
fn over_label(r: &Failure<'_>) -> String {
    let mut over: Vec<&str> = Vec::new();
    if r.complexity {
        over.push("complexity");
    }
    match r.fix {
        Some(CrapFix::MoreCoverage { .. }) => over.push("CRAP (coverage)"),
        Some(CrapFix::ReduceComplexity) => over.push("CRAP (complexity)"),
        None => {}
    }
    over.join(", ")
}

/// A function's name, or `<closure>` for one that has none.
fn display_name(f: &FunctionMetrics) -> String {
    if f.name.is_empty() {
        "<closure>".to_string()
    } else {
        f.name.clone()
    }
}

/// The report for the functions that fail the gate, worst first.
pub(crate) fn render(
    failing: &[Risk<'_>],
    limits: &Thresholds,
    baseline: Option<&Baseline>,
    record: &Record,
) -> String {
    let (complexity_on, crap_on) = (limits.complexity > 0, limits.crap > 0.0);
    let failures: Vec<Failure<'_>> = failing.iter().map(|r| Failure::new(r, limits)).collect();

    let summary = headline(&failures, limits);

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

    let rows: Vec<Vec<String>> = failures
        .iter()
        .map(|r| {
            let f = r.function;
            let mut row = vec![
                display_name(f),
                format!("{}:{}", f.file, f.line),
                f.complexity.to_string(),
                covered_cell(f),
                f.crap
                    .map_or_else(|| "n/a".to_string(), |c| format!("{c:.1}")),
                over_label(r),
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
        resolution(limits, baseline.is_some(), record)
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

    const fn limits(complexity: u32, crap: f64) -> Thresholds {
        Thresholds {
            complexity,
            crap,
            include_tests: false,
        }
    }

    const LIMITS: Thresholds = limits(15, 30.0);

    fn record() -> Record {
        Record {
            command: "jci-coverage complexity --check-only --max-complexity 15 --max-crap 30 --write-baseline"
                .into(),
            path: ".jci-coverage-baseline".into(),
        }
    }

    /// The Fix cell of the row for `name`.
    fn fix_of(out: &str, name: &str) -> String {
        let row = out
            .lines()
            .find(|l| l.starts_with('|') && l.contains(&format!("| {name} ")))
            .unwrap_or_else(|| panic!("no row for {name}:\n{out}"));
        row.trim_end_matches('|')
            .rsplit('|')
            .next()
            .expect("cell")
            .trim()
            .to_string()
    }

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
        let out = render(&risks, &LIMITS, None, &record());
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
        let out = render(&risks, &LIMITS, None, &record());
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
                && rows[2].contains("71/75 (94%)")
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
        let out = render(&risks, &LIMITS, None, &record());
        assert_eq!(fix_of(&out, "complex"), "reduce complexity below 15");
        assert_eq!(fix_of(&out, "untested"), "cover 1 more line (1 of 26)");
        assert_eq!(
            fix_of(&out, "hopeless"),
            "reduce complexity below 15",
            "well covered, so simplifying is all it needs"
        );
    }

    #[test]
    fn a_function_over_both_limits_and_untested_needs_both_fixes() {
        // Complexity 20 against 15, and 0 of 26 lines covered: once simplified
        // to 14 its CRAP is 14 × 14 + 14 = 210, so it still needs coverage.
        let f = scored("both", 20, 26, 0);
        let out = render(&[risk(&f, true, true)], &LIMITS, None, &record());
        assert_eq!(
            fix_of(&out, "both"),
            "reduce complexity below 15, then cover 15 more lines (15 of 26)"
        );
    }

    #[test]
    fn the_coverage_asked_for_after_simplifying_uses_the_stricter_limit() {
        // The CRAP limit (10) is stricter than the complexity limit (15): the
        // function has to come down to 9, and be covered enough at 9.
        let f = scored("tight", 20, 20, 0);
        let out = render(&[risk(&f, true, true)], &limits(15, 10.0), None, &record());
        let fix = fix_of(&out, "tight");
        assert!(
            fix.starts_with("reduce complexity below 15, then cover "),
            "{fix}"
        );
        // 9 × 9 × (1 − k/20)³ + 9 < 10  =>  (1 − k/20)³ < 1/81  =>  k > 15.4.
        assert!(fix.ends_with("(16 of 20)"), "{fix}");
    }

    #[test]
    fn a_crap_failure_that_tests_cannot_fix_says_why() {
        let f = scored("big", 40, 50, 50);
        let out = render(&[risk(&f, false, true)], &limits(0, 30.0), None, &record());
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
        let out = render(&risks, &LIMITS, None, &record());
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
        let out = render(&[risk(&f, true, false)], &limits(15, 0.0), None, &record());
        assert!(!out.contains("CRAP (max"), "{out}");
        assert!(!out.contains("over the CRAP limit"), "{out}");
        assert!(out.contains("over the complexity limit (15)"), "{out}");
    }

    #[test]
    fn a_function_with_no_coverage_data_shows_n_a() {
        let f = func("bare", 20, 0, 0, None);
        let out = render(&[risk(&f, true, false)], &LIMITS, None, &record());
        let row = out.lines().find(|l| l.contains("bare")).expect("row");
        assert!(row.contains("n/a"), "{row}");
    }

    #[test]
    fn coverage_is_rounded_down_so_only_full_coverage_reads_100() {
        let cell = |lines, covered| covered_cell(&scored("f", 20, lines, covered));
        assert_eq!(cell(1000, 996), "996/1000 (99%)");
        assert_eq!(cell(300, 1), "1/300 (<1%)");
        assert_eq!(cell(26, 0), "0/26 (0%)");
        assert_eq!(cell(50, 50), "50/50 (100%)");
        assert_eq!(cell(75, 71), "71/75 (94%)");
        assert_eq!(covered_cell(&func("f", 20, 0, 0, None)), "n/a");
    }

    #[test]
    fn a_closure_is_named_as_one() {
        let f = scored("", 20, 10, 10);
        let out = render(&[risk(&f, true, false)], &LIMITS, None, &record());
        assert!(out.contains("<closure>"), "{out}");
    }

    #[test]
    fn the_report_ends_with_the_ways_to_resolve_a_failure() {
        let f = scored("f", 20, 10, 10);
        let out = render(&[risk(&f, true, false)], &LIMITS, None, &record());
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
    fn the_baseline_command_is_the_one_given_and_run_from_the_repo_root() {
        let f = scored("f", 20, 10, 10);
        let custom = Record {
            command: "jci-coverage complexity --check-only --file cov.xml --rules mccabe --max-complexity 15 --write-baseline ci/b.bin"
                .into(),
            path: "ci/b.bin".into(),
        };
        let out = render(&[risk(&f, true, false)], &limits(15, 0.0), None, &custom);
        let tail = out.split("How to resolve").nth(1).expect("section");
        assert!(tail.contains("--file cov.xml --rules mccabe"), "{tail}");
        assert!(tail.contains("repository root"), "{tail}");
        assert!(
            tail.contains("jci-coverage report --format cobertura"),
            "{tail}"
        );
        assert!(
            tail.contains("commit ci/b.bin") && tail.contains("--baseline ci/b.bin"),
            "{tail}"
        );
        assert!(tail.contains("baseline: ci/b.bin"), "{tail}");
    }

    #[test]
    fn the_resolution_text_only_offers_what_applies() {
        let f = scored("f", 20, 10, 10);
        let record = Record {
            command: "jci-coverage complexity --check-only --max-complexity 15 --write-baseline"
                .into(),
            path: ".jci-coverage-baseline".into(),
        };
        let out = render(&[risk(&f, true, false)], &limits(15, 0.0), None, &record);
        let tail = out.split("How to resolve").nth(1).expect("section");
        assert!(
            !tail.contains("Add tests"),
            "complexity-only: tests do not help\n{tail}"
        );
        assert!(!tail.contains("--max-crap"), "{tail}");
    }

    #[test]
    fn a_function_worse_than_its_baseline_says_which_score_is_worse() {
        // Only the complexity has grown; the CRAP is as recorded.
        let before = func("rewrite", 29, 40, 40, Some(29.0));
        let now = func("rewrite", 31, 40, 40, Some(29.0));
        let baseline = Baseline::from_functions([&before], crate::complexity::counter::Rules::Rca);
        let out = render(
            &[risk(&now, true, false)],
            &LIMITS,
            Some(&baseline),
            &record(),
        );
        assert!(
            out.contains("worse than the baseline: complexity 31 > 29"),
            "{out}"
        );
        assert!(
            !out.contains("CRAP 29.0 >"),
            "only the score that is worse: {out}"
        );
        assert!(!out.contains("; CRAP"), "{out}");
        let tail = out.split("How to resolve").nth(1).expect("section");
        assert!(
            tail.contains("Regenerate"),
            "with a baseline in use: {tail}"
        );
    }

    #[test]
    fn a_crap_that_got_worse_or_became_known_is_named() {
        let before = func("g", 20, 10, 10, Some(20.0));
        let unknown = func("h", 20, 0, 0, None);
        let baseline =
            Baseline::from_functions([&before, &unknown], crate::complexity::counter::Rules::Rca);

        let worse = func("g", 20, 10, 5, Some(41.5));
        let out = render(
            &[risk(&worse, false, true)],
            &limits(0, 30.0),
            Some(&baseline),
            &record(),
        );
        assert!(
            out.contains("worse than the baseline: CRAP 41.5 > 20.0"),
            "{out}"
        );

        let known = func("h", 20, 10, 10, Some(20.0));
        let out = render(
            &[risk(&known, false, true)],
            &limits(0, 10.0),
            Some(&baseline),
            &record(),
        );
        assert!(out.contains("CRAP 20.0, none recorded"), "{out}");
    }

    #[test]
    fn a_function_not_in_the_baseline_is_new() {
        let other = scored("other", 29, 40, 40);
        let new = scored("fresh", 31, 40, 40);
        let baseline = Baseline::from_functions([&other], crate::complexity::counter::Rules::Rca);
        let out = render(
            &[risk(&new, true, false)],
            &LIMITS,
            Some(&baseline),
            &record(),
        );
        assert!(out.contains("not in the baseline"), "{out}");
    }
}
