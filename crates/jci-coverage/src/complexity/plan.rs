//! Join a Cobertura report's methods to the source's functions, and decide what
//! to write back: each method's complexity, which duplicate instantiations to
//! merge, and the roll-up totals.
//!
//! A method is matched to a function by the file it is in and the line it
//! starts on: `cargo llvm-cov` reports a function at its `fn` keyword line and a
//! closure at its first token. Closures that start on the same line are told
//! apart by their `{closure#N}` indexes.

use std::collections::HashMap;

use crate::complexity::{
    cobertura::{Edit, MethodEdit, Report, Rollup},
    counter::{Entry, Rules},
};

/// The analysed functions of each source file, keyed by the filename the
/// report gives. A file that could not be read or parsed is absent.
pub(crate) type Analyses = HashMap<String, Vec<Entry>>;

/// What to do with the several `<method>` entries `cargo llvm-cov` writes for
/// one generic function, one per instantiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Duplicates {
    /// One entry per function: complexity is a property of the source, so it
    /// should not depend on how many instantiations the tests happen to make.
    Merge,
    /// Leave every instantiation as `cargo llvm-cov` wrote it. OtterWise sums
    /// every entry, so a generic function counts once per instantiation.
    Keep,
}

/// What to do with a method that cannot be tied to a function in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Unmatched {
    /// Leave its complexity at `0`.
    Keep,
    /// Count it as `1`, the complexity of a function with no decisions.
    One,
    /// Fail the run.
    Fail,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Options {
    pub(crate) rules: Rules,
    pub(crate) duplicates: Duplicates,
    pub(crate) unmatched: Unmatched,
    /// Write class, package and root totals as well as method values.
    pub(crate) rollup: bool,
    /// Count `#[cfg(test)]` code and `#[test]` functions as `0`.
    pub(crate) exclude_tests: bool,
}

/// A method that was not matched, for the summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnmatchedMethod {
    pub(crate) file: String,
    pub(crate) line: u32,
    pub(crate) name: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    /// Methods in the report.
    pub(crate) methods: usize,
    /// Duplicate instantiations removed by merging.
    pub(crate) merged_away: usize,
    /// Methods left that were matched to a function.
    pub(crate) matched: usize,
    /// Methods left that were not.
    pub(crate) unmatched: Vec<UnmatchedMethod>,
    /// Sum of the complexity of the methods left.
    pub(crate) total: u32,
    /// Each method left, with its complexity, in document order.
    scored: Vec<(u32, String)>,
}

impl Summary {
    pub(crate) fn max(&self) -> Option<u32> {
        self.scored.iter().map(|(c, _)| *c).max()
    }

    /// The `n` most complex methods, most complex first.
    pub(crate) fn top(&self, n: usize) -> Vec<(u32, String)> {
        let mut v = self.scored.clone();
        v.sort_by_key(|(c, _)| std::cmp::Reverse(*c));
        v.truncate(n);
        v
    }

    /// Methods left after merging.
    pub(crate) fn kept(&self) -> usize {
        self.scored.len()
    }
}

#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) edit: Edit,
    pub(crate) summary: Summary,
}

/// The `{closure#N}` indexes a method name ends with, outermost first, or
/// `None` for a function. Only the end of the name counts: `{closure#N}` also
/// appears inside the generic arguments of ordinary functions.
pub(crate) fn closure_path(name: &str) -> Option<Vec<u32>> {
    let mut rest = name;
    let mut path = Vec::new();
    while let Some(head) = rest.strip_suffix('}') {
        let Some(open) = head.rfind("::{closure#") else {
            break;
        };
        let Ok(index) = head[open + "::{closure#".len()..].parse::<u32>() else {
            break;
        };
        path.push(index);
        rest = &rest[..open];
    }
    path.reverse();
    (!path.is_empty()).then_some(path)
}

/// `name` without its generic arguments. The self type and trait of a qualified
/// path (`<u8 as T>::f`) are kept, since `impl T for u8` and `impl T for u16`
/// are different functions, but generic arguments inside them are dropped.
pub(crate) fn strip_generics(name: &str) -> String {
    // One flag per open `<`: true when it opens generic arguments.
    let mut open: Vec<bool> = Vec::new();
    let mut out = String::with_capacity(name.len());
    let mut prev = None::<char>;
    for c in name.chars() {
        let dropping = open.iter().any(|&generic| generic);
        match c {
            '<' => {
                let generic = prev.is_some_and(|p| p.is_alphanumeric() || p == '_' || p == ':');
                open.push(generic);
                if !generic && !dropping {
                    out.push(c);
                }
            }
            '>' => {
                if open.pop() == Some(false) && !open.iter().any(|&generic| generic) {
                    out.push(c);
                }
            }
            _ if !dropping => out.push(c),
            _ => {}
        }
        prev = Some(c);
    }
    out
}

/// The final identifier of a function's path, without generic arguments.
fn function_name(name: &str) -> &str {
    let name = name.trim_end_matches("::");
    name.rsplit("::").next().unwrap_or(name)
}

/// The closure with index `path` that covers `line`, for when no closure starts
/// on it: `cargo llvm-cov` reports a closure at its first counted line, which
/// can be later than the line the closure starts on. A function is never found
/// this way, and neither is a closure whose index does not agree. When closures
/// of one index nest through a nested function, the innermost wins.
fn covering_closure<'a>(
    entries: Option<&'a Vec<Entry>>,
    path: &[u32],
    line: usize,
) -> Option<&'a Entry> {
    if path.is_empty() {
        return None;
    }
    entries?
        .iter()
        .filter(|e| e.path == path && e.line <= line && line <= e.end_line)
        .max_by_key(|e| e.line)
}

/// The entries of one file by (start line, closure path).
type EntryIndex<'a> = HashMap<(usize, Vec<u32>), Vec<&'a Entry>>;

/// Work out the edit for `report` from the analysed sources.
pub(crate) fn plan(report: &Report, analyses: &Analyses, opts: &Options) -> Plan {
    // (line, closure path) -> entries, per file, in source order. Several
    // functions can start on one line; they are told apart by name.
    let mut index: HashMap<&str, EntryIndex> = HashMap::new();
    for (file, entries) in analyses {
        let by_key = index.entry(file.as_str()).or_default();
        for e in entries {
            by_key.entry((e.line, e.path.clone())).or_default().push(e);
        }
    }

    // Complexity of each method, and whether it was matched.
    let scored: Vec<(u32, bool)> = report
        .methods
        .iter()
        .map(|m| {
            let file = report.classes[m.class].filename.as_str();
            let path = closure_path(&m.name).unwrap_or_default();
            let found = index
                .get(file)
                .and_then(|by_key| by_key.get(&(m.line as usize, path.clone())))
                .and_then(|entries| {
                    let wanted = function_name(&m.name);
                    entries
                        .iter()
                        .find(|e| path.is_empty() && e.name == wanted)
                        .or_else(|| entries.first())
                })
                .copied()
                .or_else(|| covering_closure(analyses.get(file), &path, m.line as usize));
            match found {
                Some(e) if opts.exclude_tests && e.is_test => (0, true),
                Some(e) => (e.counts.complexity(opts.rules), true),
                None => (u32::from(opts.unmatched == Unmatched::One), false),
            }
        })
        .collect();

    // With merging, the first method of each function stands for the rest.
    let mut first_of: HashMap<(usize, u32, String), usize> = HashMap::new();
    let mut hits_of: HashMap<usize, u64> = HashMap::new();
    let mut size_of: HashMap<usize, usize> = HashMap::new();
    let mut group_of: Vec<usize> = Vec::with_capacity(report.methods.len());
    for (i, m) in report.methods.iter().enumerate() {
        let first = match opts.duplicates {
            Duplicates::Keep => i,
            Duplicates::Merge => *first_of
                .entry((m.class, m.line, strip_generics(&m.name)))
                .or_insert(i),
        };
        *hits_of.entry(first).or_default() += m.hits;
        *size_of.entry(first).or_default() += 1;
        group_of.push(first);
    }

    let mut summary = Summary {
        methods: report.methods.len(),
        ..Summary::default()
    };
    let mut class_totals = vec![0u32; report.classes.len()];
    let mut methods = Vec::with_capacity(report.methods.len());

    for (i, m) in report.methods.iter().enumerate() {
        if group_of[i] != i {
            summary.merged_away += 1;
            methods.push(MethodEdit::Drop);
            continue;
        }
        let (complexity, matched) = scored[i];
        let merged = size_of[&i] > 1;
        methods.push(MethodEdit::Set {
            complexity,
            hits: merged.then(|| hits_of[&i]),
        });

        summary.total += complexity;
        class_totals[m.class] += complexity;
        summary.scored.push((complexity, m.name.clone()));
        if matched {
            summary.matched += 1;
        } else {
            summary.unmatched.push(UnmatchedMethod {
                file: report.classes[m.class].filename.clone(),
                line: m.line,
                name: m.name.clone(),
            });
        }
    }

    let rollup = opts.rollup.then(|| {
        let mut packages = vec![0u32; report.package_count];
        for (class, total) in report.classes.iter().zip(&class_totals) {
            packages[class.package] += total;
        }
        Rollup {
            classes: class_totals.clone(),
            root: packages.iter().sum(),
            packages,
        }
    });

    Plan {
        edit: Edit { methods, rollup },
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::complexity::{
        cobertura::{Class, Edit, Method, MethodEdit, Report},
        counter::{Rules, analyze},
    };

    // line 1: plain (1) | 3-5: two arms + `?` | 7: closure fn with a nested closure
    const LIB: &str = "\
fn plain() {}

fn pick(x: Option<i32>) -> Option<i32> {
    let v = x?;
    match v { 0 => None, _ => Some(v) }
}

fn outer() -> i32 { let f = |a: i32| { let g = |b: i32| if b > 0 { b } else { 0 }; g(a) }; f(1) }

#[cfg(test)]
mod tests {
    #[test]
    fn t() { if true {} }
}
";

    fn method(class: usize, name: &str, line: u32, hits: u64) -> Method {
        Method {
            class,
            name: name.to_string(),
            line,
            hits,
        }
    }

    fn report(methods: Vec<Method>) -> Report {
        Report {
            package_count: 2,
            classes: vec![
                Class {
                    filename: "src/lib.rs".to_string(),
                    package: 0,
                    lines: Vec::new(),
                },
                Class {
                    filename: "src/gone.rs".to_string(),
                    package: 1,
                    lines: Vec::new(),
                },
            ],
            methods,
        }
    }

    fn analyses() -> Analyses {
        Analyses::from([("src/lib.rs".to_string(), analyze(LIB).expect("parses"))])
    }

    fn opts() -> Options {
        Options {
            rules: Rules::Rca,
            duplicates: Duplicates::Merge,
            unmatched: Unmatched::Keep,
            rollup: true,
            exclude_tests: false,
        }
    }

    fn sets(plan: &Plan) -> Vec<Option<(u32, Option<u64>)>> {
        plan.edit
            .methods
            .iter()
            .map(|m| match m {
                MethodEdit::Drop => None,
                MethodEdit::Set { complexity, hits } => Some((*complexity, *hits)),
            })
            .collect()
    }

    #[test]
    fn closure_path_reads_the_closure_suffix_of_a_name() {
        assert_eq!(closure_path("a::b::f"), None);
        assert_eq!(closure_path("a::b::f::{closure#0}"), Some(vec![0]));
        assert_eq!(
            closure_path("a::f::{closure#1}::{closure#0}"),
            Some(vec![1, 0])
        );
        assert_eq!(
            closure_path("<a::X as a::T>::run::{closure#2}"),
            Some(vec![2])
        );
        // `{closure#N}` inside generic arguments does not make a function a closure.
        assert_eq!(closure_path("a::f::<a::g::{closure#1}>"), None);
        assert_eq!(
            closure_path("a::f::<{closure#1}>::{closure#0}"),
            Some(vec![0])
        );
    }

    #[test]
    fn strip_generics_removes_balanced_angle_groups() {
        assert_eq!(strip_generics("a::f::<u8>"), "a::f::");
        assert_eq!(strip_generics("a::f::<Vec<u8>, &'a str>::g"), "a::f::::g");
        assert_eq!(strip_generics("a::f"), "a::f");
    }

    #[test]
    fn strip_generics_keeps_the_self_type_and_trait_of_a_qualified_path() {
        assert_eq!(strip_generics("<a::X as a::T>::run"), "<a::X as a::T>::run");
        assert_eq!(
            strip_generics("<a::X<u8> as a::T<i8>>::run::<u16>"),
            "<a::X as a::T>::run::"
        );
        assert_eq!(strip_generics("<u8 as a::T>::f"), "<u8 as a::T>::f");
    }

    #[test]
    fn distinct_impls_on_one_line_are_not_merged() {
        let r = report(vec![
            method(0, "<u8 as x::T>::f", 1, 1),
            method(0, "<u16 as x::T>::f", 1, 0),
            method(0, "<u16 as x::T>::f", 1, 2),
        ]);
        let plan = plan(&r, &analyses(), &opts());
        assert_eq!(plan.summary.merged_away, 1);
        assert_eq!(sets(&plan), vec![Some((1, None)), Some((1, Some(2))), None]);
    }

    #[test]
    fn functions_sharing_a_source_line_get_their_own_value() {
        let src = "fn a() {} fn b() { if true {} }\n";
        let analyses = Analyses::from([("src/lib.rs".to_string(), analyze(src).expect("parses"))]);
        let r = report(vec![method(0, "x::a", 1, 1), method(0, "x::b", 1, 1)]);
        let plan = plan(&r, &analyses, &opts());
        assert_eq!(sets(&plan), vec![Some((1, None)), Some((2, None))]);
    }

    #[test]
    fn functions_and_closures_get_the_complexity_of_their_source() {
        let r = report(vec![
            method(0, "x::plain", 1, 1),
            method(0, "x::pick", 3, 1),
            method(0, "x::outer", 8, 1),
            method(0, "x::outer::{closure#0}", 8, 1),
            method(0, "x::outer::{closure#0}::{closure#0}", 8, 1),
        ]);
        let plan = plan(&r, &analyses(), &opts());
        // plain 1; pick 1 + `?` + two arms = 4; outer 1; its closure 1; inner closure 2.
        assert_eq!(
            sets(&plan),
            vec![
                Some((1, None)),
                Some((4, None)),
                Some((1, None)),
                Some((1, None)),
                Some((2, None))
            ]
        );
        assert_eq!(plan.summary.matched, 5);
        assert!(plan.summary.unmatched.is_empty());
        assert_eq!(plan.summary.total, 9);
    }

    /// `cargo llvm-cov` reports a closure at the first line that carries a count,
    /// which is not always the line the closure starts on.
    #[test]
    fn a_multi_line_closure_is_found_by_the_lines_it_covers() {
        const SRC: &str = "\
fn f(v: &[i32]) -> Vec<Wrapper> {
    v.iter().map(|x| Wrapper {
        a: if *x > 0 { 1 } else { 0 },
    }).collect()
}

fn g() {}
";
        let analyses = Analyses::from([("src/lib.rs".to_string(), analyze(SRC).expect("parses"))]);
        let r = report(vec![
            method(0, "x::f::{closure#0}", 3, 1),
            method(0, "x::f", 1, 1),
            method(0, "x::g", 7, 1),
        ]);
        let plan = plan(&r, &analyses, &opts());
        assert_eq!(
            sets(&plan),
            vec![Some((2, None)), Some((1, None)), Some((1, None))]
        );
        assert!(
            plan.summary.unmatched.is_empty(),
            "{:?}",
            plan.summary.unmatched
        );
    }

    #[test]
    fn the_covered_line_fallback_needs_the_closure_index_to_agree() {
        const SRC: &str = "\
fn f(v: &[i32]) {
    v.iter().for_each(|x| {
        let _ = x;
    });
}
";
        let analyses = Analyses::from([("src/lib.rs".to_string(), analyze(SRC).expect("parses"))]);
        // closure#1 does not exist, and a plain function is never matched by a covered line
        let r = report(vec![
            method(0, "x::f::{closure#1}", 3, 1),
            method(0, "x::other", 3, 1),
        ]);
        let plan = plan(&r, &analyses, &opts());
        assert_eq!(plan.summary.unmatched.len(), 2);
    }

    #[test]
    fn nested_closures_found_by_covered_line_are_told_apart_by_their_index_path() {
        const SRC: &str = "\
fn f() {
    let a = |x: i32| {
        let b = |y: i32| {
            y + 1
        };
        b(x)
    };
    a(1);
}
";
        let analyses = Analyses::from([("src/lib.rs".to_string(), analyze(SRC).expect("parses"))]);
        // llvm-cov reports both at line 4; the nested one has the longer index path.
        let r = report(vec![
            method(0, "x::f::{closure#0}", 4, 1),
            method(0, "x::f::{closure#0}::{closure#0}", 4, 1),
        ]);
        let plan = plan(&r, &analyses, &opts());
        assert_eq!(plan.summary.matched, 2);
    }

    #[test]
    fn rules_change_the_values() {
        let r = report(vec![method(0, "x::pick", 3, 1)]);
        let mut o = opts();
        o.rules = Rules::Mccabe;
        // 1 + (2 arms - 1), no `?`
        assert_eq!(sets(&plan(&r, &analyses(), &o)), vec![Some((2, None))]);
    }

    #[test]
    fn instantiations_of_one_function_merge_into_one_method_with_summed_hits() {
        let r = report(vec![
            method(0, "x::pick::<u8>", 3, 2),
            method(0, "x::pick::<u16>", 3, 0),
            method(0, "x::plain", 1, 1),
            method(0, "x::pick::<u32>", 3, 5),
        ]);
        let plan = plan(&r, &analyses(), &opts());
        assert_eq!(
            sets(&plan),
            vec![Some((4, Some(7))), None, Some((1, None)), None]
        );
        assert_eq!(plan.summary.merged_away, 2);
        assert_eq!(plan.summary.total, 5, "pick counted once");
    }

    #[test]
    fn keep_leaves_every_instantiation_as_it_is() {
        let r = report(vec![
            method(0, "x::pick::<u8>", 3, 2),
            method(0, "x::pick::<u16>", 3, 0),
        ]);
        let mut o = opts();
        o.duplicates = Duplicates::Keep;
        let plan = plan(&r, &analyses(), &o);
        assert_eq!(sets(&plan), vec![Some((4, None)), Some((4, None))]);
        assert_eq!(plan.summary.merged_away, 0);
        assert_eq!(plan.summary.total, 8, "counted once per instantiation");
    }

    #[test]
    fn nested_closures_that_start_on_one_line_stay_separate_when_merging() {
        let r = report(vec![
            method(0, "x::outer::{closure#0}", 8, 1),
            method(0, "x::outer::{closure#0}::{closure#0}", 8, 1),
        ]);
        let plan = plan(&r, &analyses(), &opts());
        assert_eq!(sets(&plan), vec![Some((1, None)), Some((2, None))]);
    }

    #[test]
    fn methods_with_no_source_follow_the_unmatched_policy() {
        let r = report(vec![
            method(0, "x::generated", 99, 0),
            method(1, "x::gone::f", 1, 0),
            method(0, "x::plain", 1, 1),
        ]);
        let keep = plan(&r, &analyses(), &opts());
        assert_eq!(
            sets(&keep),
            vec![Some((0, None)), Some((0, None)), Some((1, None))]
        );
        assert_eq!(keep.summary.unmatched.len(), 2);
        assert_eq!(
            keep.summary.unmatched[1],
            UnmatchedMethod {
                file: "src/gone.rs".to_string(),
                line: 1,
                name: "x::gone::f".to_string()
            }
        );
        let mut o = opts();
        o.unmatched = Unmatched::One;
        assert_eq!(
            sets(&plan(&r, &analyses(), &o)),
            vec![Some((1, None)), Some((1, None)), Some((1, None))]
        );
    }

    #[test]
    fn test_code_counts_as_zero_when_excluded() {
        // `t` is the #[test] function at line 13.
        let r = report(vec![
            method(0, "x::tests::t", 13, 1),
            method(0, "x::plain", 1, 1),
        ]);
        assert_eq!(
            sets(&plan(&r, &analyses(), &opts())),
            vec![Some((2, None)), Some((1, None))]
        );
        let mut o = opts();
        o.exclude_tests = true;
        assert_eq!(
            sets(&plan(&r, &analyses(), &o)),
            vec![Some((0, None)), Some((1, None))]
        );
    }

    #[test]
    fn rollup_adds_up_the_methods_that_remain() {
        let r = report(vec![
            method(0, "x::plain", 1, 1),
            method(0, "x::pick::<u8>", 3, 1),
            method(0, "x::pick::<u16>", 3, 1),
            method(1, "x::gone::f", 1, 0),
        ]);
        let mut o = opts();
        o.unmatched = Unmatched::One;
        let plan = plan(&r, &analyses(), &o);
        let rollup = plan.edit.rollup.expect("on");
        assert_eq!(rollup.classes, vec![1 + 4, 1]);
        assert_eq!(rollup.packages, vec![5, 1]);
        assert_eq!(rollup.root, 6);

        let mut o = opts();
        o.rollup = false;
        assert!(plan_without_rollup(&r, &o));
    }

    fn plan_without_rollup(r: &Report, o: &Options) -> bool {
        plan(r, &analyses(), o).edit.rollup.is_none()
    }

    #[test]
    fn the_summary_names_the_most_complex_functions() {
        let r = report(vec![
            method(0, "x::plain", 1, 1),
            method(0, "x::pick", 3, 1),
            method(0, "x::outer::{closure#0}::{closure#0}", 8, 1),
        ]);
        let s = plan(&r, &analyses(), &opts()).summary;
        assert_eq!(
            s.top(2),
            vec![
                (4, "x::pick".to_string()),
                (2, "x::outer::{closure#0}::{closure#0}".to_string())
            ]
        );
        assert_eq!(s.max(), Some(4));
        assert_eq!(s.methods, 3);
    }

    #[test]
    fn an_empty_report_plans_nothing() {
        let plan = plan(&report(vec![]), &Analyses::new(), &opts());
        assert!(plan.edit.methods.is_empty());
        assert_eq!(plan.summary.total, 0);
        assert_eq!(plan.summary.max(), None);
        let rollup = plan.edit.rollup.expect("on");
        assert_eq!(
            (rollup.classes, rollup.packages, rollup.root),
            (vec![0, 0], vec![0, 0], 0)
        );
    }

    #[test]
    fn an_edit_always_has_one_entry_per_method() {
        let r = report(vec![
            method(0, "x::plain", 1, 1),
            method(0, "x::plain", 1, 1),
        ]);
        let plan: Plan = plan(&r, &analyses(), &opts());
        let edit: &Edit = &plan.edit;
        assert_eq!(edit.methods.len(), r.methods.len());
    }
}
