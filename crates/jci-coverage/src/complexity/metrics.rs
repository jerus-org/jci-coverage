//! Complexity and CRAP of each function in the source, one score per function.
//!
//! OtterWise's own figures are totals and per-line averages, which a few very
//! complex functions can hide among many trivial ones. These scores are per
//! function, so a risky one can be named.
//!
//! CRAP is `complexity² × (1 − coverage)³ + complexity`. `cargo llvm-cov` gives
//! every `<method>` a single line, so a method's own `line-rate` is only ever
//! `0` or `1` and says nothing about how much of the function ran. Coverage here
//! is therefore taken from the class-level lines: the share of the function's
//! own executable lines that were hit. A line belongs to the innermost function
//! or closure containing it, so a closure's lines count for the closure and not
//! for the function around it, as complexity is already split.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashSet},
};

use crate::complexity::{
    cobertura::Report,
    counter::{Entry, Rules},
    plan::Analyses,
};

/// The scores of one function or closure.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FunctionMetrics {
    pub(crate) file: String,
    /// The function's own name, or empty for a closure.
    pub(crate) name: String,
    /// The line it starts on.
    pub(crate) line: usize,
    pub(crate) complexity: u32,
    /// Executable lines that belong to it.
    pub(crate) lines: usize,
    /// Of those, how many were hit.
    pub(crate) covered: usize,
    /// `None` when none of its lines were counted, so coverage is unknown.
    pub(crate) crap: Option<f64>,
    /// In `#[cfg(test)]` code, or a `#[test]` function.
    pub(crate) is_test: bool,
}

/// CRAP for a function of `complexity` with `covered` of its `lines` hit.
/// `lines` must not be `0`.
pub(crate) fn crap(complexity: u32, covered: usize, lines: usize) -> f64 {
    let uncovered = 1.0 - covered as f64 / lines as f64;
    let c = f64::from(complexity);
    c * c * uncovered.powi(3) + c
}

/// How high a score has to be before a function is called risky. A threshold of
/// `0` turns that check off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Thresholds {
    pub(crate) complexity: u32,
    pub(crate) crap: f64,
    /// Also judge `#[cfg(test)]` code and `#[test]` functions.
    pub(crate) include_tests: bool,
}

/// A function at or over a threshold, and which one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Risk<'a> {
    pub(crate) function: &'a FunctionMetrics,
    pub(crate) complexity: bool,
    pub(crate) crap: bool,
}

/// The functions at or over a threshold, the worst first. A function whose
/// coverage is unknown is judged on complexity alone, since CRAP is never below
/// it, and is ranked by it.
pub(crate) fn risky<'a>(functions: &'a [FunctionMetrics], t: &Thresholds) -> Vec<Risk<'a>> {
    let mut risks: Vec<Risk<'a>> = functions
        .iter()
        .filter(|f| t.include_tests || !f.is_test)
        .filter_map(|function| {
            let complexity = t.complexity > 0 && function.complexity >= t.complexity;
            let crap = t.crap > 0.0 && function.crap.is_some_and(|c| c >= t.crap);
            (complexity || crap).then_some(Risk {
                function,
                complexity,
                crap,
            })
        })
        .collect();
    let severity = |r: &Risk<'_>| {
        r.function
            .crap
            .unwrap_or_else(|| f64::from(r.function.complexity))
    };
    risks.sort_by(|a, b| {
        severity(b)
            .total_cmp(&severity(a))
            .then(b.function.complexity.cmp(&a.function.complexity))
            .then_with(|| a.function.file.cmp(&b.function.file))
            .then(a.function.line.cmp(&b.function.line))
            .then_with(|| a.function.name.cmp(&b.function.name))
    });
    risks
}

/// The index of the innermost entry containing `line`: the smallest span, then
/// the more deeply nested closure (the shallower on a single line), then the
/// later one.
fn innermost(entries: &[Entry], line: usize) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.line <= line && line <= e.end_line)
        .min_by_key(|(i, e)| {
            let span = e.end_line - e.line;
            // On a one-line function the closure cannot be told from it, so the
            // function keeps the line; otherwise the deeper closure owns it.
            let depth = if span == 0 {
                e.path.len() as isize
            } else {
                -(e.path.len() as isize)
            };
            (span, depth, Reverse(*i))
        })
        .map(|(i, _)| i)
}

/// The metrics of every function and closure of each analysed file, in report
/// order and then by start line. A file the report names but that was not
/// analysed gives none.
pub(crate) fn function_metrics(
    report: &Report,
    analyses: &Analyses,
    rules: Rules,
) -> Vec<FunctionMetrics> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for class in &report.classes {
        let Some(entries) = analyses.get(&class.filename) else {
            continue;
        };
        if !seen.insert(class.filename.as_str()) {
            continue;
        }
        // Each line once, with its most hits, in case a report repeats one.
        let mut hits: BTreeMap<u32, u64> = BTreeMap::new();
        for &(number, h) in &class.lines {
            let slot = hits.entry(number).or_default();
            *slot = (*slot).max(h);
        }
        let mut owned = vec![(0usize, 0usize); entries.len()];
        for (&number, &h) in &hits {
            if let Some(i) = innermost(entries, number as usize) {
                owned[i].0 += 1;
                owned[i].1 += usize::from(h > 0);
            }
        }
        let mut order: Vec<usize> = (0..entries.len()).collect();
        order.sort_by_key(|&i| (entries[i].line, entries[i].path.len()));
        for i in order {
            let e = &entries[i];
            let (lines, covered) = owned[i];
            let complexity = e.counts.complexity(rules);
            out.push(FunctionMetrics {
                file: class.filename.clone(),
                name: e.name.clone(),
                line: e.line,
                complexity,
                lines,
                covered,
                crap: (lines > 0).then(|| crap(complexity, covered, lines)),
                is_test: e.is_test,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::complexity::{
        cobertura::{Class, Report},
        counter::analyze,
    };

    /// A report whose one file `src/a.rs` has these (line, hits) pairs, and the
    /// analysis of `source` for it.
    fn setup(source: &str, lines: &[(u32, u64)]) -> (Report, Analyses) {
        let report = Report {
            package_count: 1,
            classes: vec![Class {
                filename: "src/a.rs".to_string(),
                package: 0,
                lines: lines.to_vec(),
            }],
            methods: Vec::new(),
        };
        let analyses = Analyses::from([("src/a.rs".to_string(), analyze(source).expect("parses"))]);
        (report, analyses)
    }

    fn by_name<'a>(m: &'a [FunctionMetrics], name: &str) -> &'a FunctionMetrics {
        m.iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("no function {name}: {m:?}"))
    }

    #[test]
    fn a_one_line_function_keeps_its_line_from_its_closure() {
        let src = "fn f(v: &[i32]) -> Vec<i32> { v.iter().map(|x| x + 1).collect() }\n";
        let (report, analyses) = setup(src, &[(1, 3)]);
        let m = function_metrics(&report, &analyses, Rules::Rca);
        let f = by_name(&m, "f");
        assert_eq!((f.lines, f.covered), (1, 1), "{m:?}");
        assert!(f.crap.is_some());
    }

    #[test]
    fn a_class_listed_twice_for_one_file_is_counted_once() {
        let (mut report, analyses) = setup(BRANCHY, &[(2, 1)]);
        report.classes.push(report.classes[0].clone());
        let m = function_metrics(&report, &analyses, Rules::Rca);
        assert_eq!(m.len(), 1, "{m:?}");
    }

    const BRANCHY: &str = "fn f(x: i32) -> i32 {\n    if x > 0 { 1 } else { 2 }\n}\n";

    #[test]
    fn crap_is_the_complexity_alone_when_fully_covered() {
        assert_eq!(crap(10, 10, 10), 10.0);
    }

    #[test]
    fn crap_adds_the_square_of_the_complexity_when_nothing_is_covered() {
        assert_eq!(crap(10, 0, 10), 110.0);
    }

    #[test]
    fn crap_scales_with_the_cube_of_the_uncovered_share() {
        // complexity 2, half covered: 2^2 * 0.5^3 + 2
        assert_eq!(crap(2, 2, 4), 2.5);
    }

    #[test]
    fn a_covered_function_scores_its_complexity() {
        let (report, analyses) = setup(BRANCHY, &[(1, 5), (2, 5), (3, 5)]);
        let m = function_metrics(&report, &analyses, Rules::Rca);
        let f = by_name(&m, "f");
        assert_eq!((f.complexity, f.lines, f.covered), (2, 3, 3));
        assert_eq!(f.crap, Some(2.0));
        assert_eq!((f.file.as_str(), f.line), ("src/a.rs", 1));
    }

    #[test]
    fn an_uncovered_function_scores_complexity_squared_plus_complexity() {
        let (report, analyses) = setup(BRANCHY, &[(1, 0), (2, 0), (3, 0)]);
        let f = &function_metrics(&report, &analyses, Rules::Rca)[0];
        assert_eq!((f.lines, f.covered), (3, 0));
        assert_eq!(f.crap, Some(6.0));
    }

    #[test]
    fn a_partly_covered_function_scores_between() {
        let src = "fn f(x: i32) -> i32 {\n    let a = 1;\n    let b = 2;\n    if x > 0 { a } else { b }\n}\n";
        let (report, analyses) = setup(src, &[(1, 1), (2, 1), (3, 0), (4, 0), (5, 0)]);
        let f = &function_metrics(&report, &analyses, Rules::Rca)[0];
        assert_eq!((f.lines, f.covered), (5, 2));
        // complexity 2, 2 of 5 covered: 4 * 0.6^3 + 2
        assert!((f.crap.unwrap() - 2.864).abs() < 1e-9, "{:?}", f.crap);
    }

    #[test]
    fn a_nested_closures_lines_belong_to_the_closure_not_the_function() {
        let src = "\
fn f(v: &[i32]) -> usize {
    v.iter()
        .filter(|x| {
            **x > 0 && **x < 9
        })
        .count()
}
";
        let (report, analyses) = setup(src, &[(1, 1), (2, 1), (3, 1), (4, 0), (6, 1), (7, 1)]);
        let m = function_metrics(&report, &analyses, Rules::Rca);
        let f = by_name(&m, "f");
        let closure = by_name(&m, "");
        assert_eq!((f.lines, f.covered), (4, 4), "lines 1, 2, 6 and 7");
        assert_eq!((closure.lines, closure.covered), (2, 1), "lines 3 and 4");
        assert_eq!(closure.complexity, 2, "`&&` counts in the closure");
        assert_eq!(f.complexity, 1);
    }

    #[test]
    fn a_nested_functions_lines_belong_to_the_inner_function() {
        let src = "fn outer() {\n    fn inner(x: bool) -> bool {\n        x || false\n    }\n    inner(true);\n}\n";
        let (report, analyses) = setup(src, &[(1, 1), (2, 1), (3, 1), (5, 1), (6, 1)]);
        let m = function_metrics(&report, &analyses, Rules::Rca);
        assert_eq!(by_name(&m, "outer").lines, 3, "lines 1, 5 and 6");
        assert_eq!(by_name(&m, "inner").lines, 2, "lines 2 and 3");
    }

    #[test]
    fn a_function_with_no_counted_lines_has_no_crap() {
        let (report, analyses) = setup(BRANCHY, &[]);
        let f = &function_metrics(&report, &analyses, Rules::Rca)[0];
        assert_eq!((f.lines, f.covered, f.complexity), (0, 0, 2));
        assert_eq!(f.crap, None);
    }

    #[test]
    fn test_code_is_marked_so_it_can_be_left_out() {
        let src =
            "fn real() {}\n#[cfg(test)]\nmod t {\n    #[test]\n    fn it() { if true {} }\n}\n";
        let (report, analyses) = setup(src, &[(1, 1), (5, 1)]);
        let m = function_metrics(&report, &analyses, Rules::Rca);
        assert!(!by_name(&m, "real").is_test);
        assert!(by_name(&m, "it").is_test);
    }

    #[test]
    fn a_file_the_report_names_but_that_was_not_analysed_gives_nothing() {
        let (report, _) = setup(BRANCHY, &[(1, 1)]);
        assert!(function_metrics(&report, &Analyses::new(), Rules::Rca).is_empty());
    }

    #[test]
    fn the_rule_set_changes_the_complexity() {
        let src = "fn f(x: Option<i32>) -> i32 {\n    match x {\n        Some(v) => v,\n        None => 0,\n    }\n}\n";
        let (report, analyses) = setup(src, &[(1, 1), (2, 1), (3, 1), (4, 1)]);
        let rca = function_metrics(&report, &analyses, Rules::Rca);
        let mccabe = function_metrics(&report, &analyses, Rules::Mccabe);
        assert_eq!((rca[0].complexity, mccabe[0].complexity), (3, 2));
    }

    fn metric(name: &str, complexity: u32, crap: Option<f64>, is_test: bool) -> FunctionMetrics {
        FunctionMetrics {
            file: "src/a.rs".to_string(),
            name: name.to_string(),
            line: 1,
            complexity,
            lines: 4,
            covered: 4,
            crap,
            is_test,
        }
    }

    fn thresholds(complexity: u32, crap: f64) -> Thresholds {
        Thresholds {
            complexity,
            crap,
            include_tests: false,
        }
    }

    fn names<'a>(risks: &[Risk<'a>]) -> Vec<&'a str> {
        risks.iter().map(|r| r.function.name.as_str()).collect()
    }

    #[test]
    fn a_function_at_or_over_a_threshold_is_risky() {
        let fns = [
            metric("low", 9, Some(9.0), false),
            metric("at", 10, Some(10.0), false),
            metric("crappy", 5, Some(30.0), false),
        ];
        let risks = risky(&fns, &thresholds(10, 30.0));
        assert_eq!(names(&risks), ["crappy", "at"]);
        let at = risks.iter().find(|r| r.function.name == "at").unwrap();
        assert_eq!((at.complexity, at.crap), (true, false));
        let crappy = risks.iter().find(|r| r.function.name == "crappy").unwrap();
        assert_eq!((crappy.complexity, crappy.crap), (false, true));
    }

    #[test]
    fn a_function_can_be_over_both_thresholds() {
        let fns = [metric("both", 20, Some(420.0), false)];
        let risks = risky(&fns, &thresholds(10, 30.0));
        assert_eq!((risks[0].complexity, risks[0].crap), (true, true));
    }

    #[test]
    fn a_threshold_of_zero_is_off() {
        let fns = [metric("f", 40, Some(1640.0), false)];
        assert!(risky(&fns, &thresholds(0, 0.0)).is_empty());
        assert_eq!(risky(&fns, &thresholds(0, 30.0)).len(), 1);
        assert_eq!(risky(&fns, &thresholds(10, 0.0)).len(), 1);
    }

    #[test]
    fn a_function_with_unknown_coverage_is_judged_on_complexity_only() {
        let fns = [metric("unknown", 12, None, false)];
        let risks = risky(&fns, &thresholds(10, 30.0));
        assert_eq!((risks[0].complexity, risks[0].crap), (true, false));
        assert!(risky(&fns, &thresholds(0, 30.0)).is_empty());
    }

    #[test]
    fn test_code_is_left_out_unless_asked() {
        let fns = [metric("t", 30, Some(900.0), true)];
        assert!(risky(&fns, &thresholds(10, 30.0)).is_empty());
        let with_tests = Thresholds {
            include_tests: true,
            ..thresholds(10, 30.0)
        };
        assert_eq!(risky(&fns, &with_tests).len(), 1);
    }

    #[test]
    fn the_worst_function_comes_first() {
        let fns = [
            metric("mild", 12, Some(12.0), false),
            metric("worst", 20, Some(420.0), false),
            metric("unknown", 25, None, false),
            metric("bad", 11, Some(132.0), false),
        ];
        let risks = risky(&fns, &thresholds(10, 30.0));
        assert_eq!(names(&risks), ["worst", "bad", "unknown", "mild"]);
    }
}
