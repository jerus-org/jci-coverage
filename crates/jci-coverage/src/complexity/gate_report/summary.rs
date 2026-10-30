//! The gate failure as a short markdown summary, for a PR comment or a file.
//!
//! The log report ([`super::render`]) is written for a terminal: wide, with the
//! full resolution text. This is the same analysis shortened for a comment that
//! people read on the pull request: the headline counts, one compact row per
//! failing function (capped, so the comment stays small), the baselined functions
//! whose scores have moved, a collapsed list of those that have not, the
//! resolution options in a line, and a pointer to
//! the CI log that has the full report.
//!
//! The first line is a hidden marker, so a later run can find the comment it
//! wrote and edit it in place. Names and paths come from the repository, so they
//! are put in code spans, with `|` escaped for the table, and clipped. There is no
//! link to the CI job: its log may not be readable by everyone who can read the
//! pull request, so a link would only disclose where it is.

use super::{Failure, against_baseline, display_name, fix, headline, over_label};
use crate::complexity::{
    baseline::{Baseline, Movement, kept_crap_text},
    metrics::{FunctionMetrics, Risk, Thresholds},
};

/// First line of every summary: finds the comment again so it can be updated.
pub(crate) const MARKER: &str = "<!-- jci-coverage:gate -->";

/// How many failing functions the summary lists before counting the rest.
pub(crate) const MAX_ROWS: usize = 25;

/// The longest name or path shown in a cell; longer ones are clipped.
const MAX_CELL: usize = 120;

/// `text` cut to `max` characters, with a `…` where it was cut.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// `text` as a markdown code span that is safe inside a table cell: no
/// backticks or line breaks (which would end the span or the row), and `|`
/// escaped. HTML such as a closure's `<closure>` is literal inside a code span.
fn code(text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| match c {
            '`' => '\'',
            '\\' => '/',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    format!("`{}`", clean.replace('|', "\\|"))
}

/// The "Function" cell: the name, and below it where it is.
fn function_cell(f: &FunctionMetrics) -> String {
    let name = code(&clip(&display_name(f), MAX_CELL));
    let place = code(&format!("{}:{}", clip(&f.file, MAX_CELL), f.line));
    format!("{name}<br>{place}")
}

/// The one-line "To resolve" pointer: the options that apply to the limits set.
fn resolution(limits: &Thresholds) -> String {
    let mut text =
        String::from("**To resolve:** fix each function listed by reducing its complexity");
    if limits.crap > 0.0 {
        text += " or adding tests for its own lines (the Fix column says how many are enough)";
    }
    text += "; or, to adopt the limits without fixing them all now, accept today's state with a \
            baseline (`jci-coverage complexity --write-baseline`, then commit \
            `.jci-coverage-baseline`), which leaves the functions it records allowed and fails \
            any that get worse or are new; or change the limits. \
            The CI log has the full report and the exact command.";
    text
}

/// The baselined functions that moved, as a section: how many each way, what to
/// do about them, and a capped table, the degraded ones first. Empty when none
/// moved. They stay within what the baseline accepts, so they never fail the gate.
fn moved_section(moves: &[Movement<'_>], max_rows: usize) -> String {
    if moves.is_empty() {
        return String::new();
    }
    let degraded = moves.iter().filter(|m| m.degraded()).count();
    let improved = moves.iter().filter(|m| m.improved()).count();
    let mut counts = Vec::new();
    if degraded > 0 {
        counts.push(format!("{degraded} degraded"));
    }
    if improved > 0 {
        counts.push(format!("{improved} improved"));
    }
    let mut out = format!(
        "\n#### Baselined functions that moved\n\n{}.",
        counts.join(", ")
    );
    if degraded > 0 {
        out += " A degraded function is still accepted by the baseline; reworking it brings it back to what was recorded.";
    }
    if improved > 0 {
        out += " Record the baseline again (`jci-coverage complexity --write-baseline`) to lock an improvement in.";
    }
    out += "\n\n| Function | Complexity | CRAP |\n|---|---|---|\n";
    for m in moves.iter().take(max_rows) {
        out += &format!(
            "| {} | {} | {} |\n",
            function_cell(m.function),
            m.complexity_text(),
            m.crap_text(),
        );
    }
    if moves.len() > max_rows {
        out += &format!("\n_{} more not listed._\n", moves.len() - max_rows);
    }
    out
}

/// The baselined functions over a limit whose scores are as recorded, in a
/// collapsed table: they are not news, but the reader can see what the
/// baseline lets through. Those that moved have their own table, so they are
/// not repeated here.
fn unchanged_section(unchanged: &[Risk<'_>], max_rows: usize) -> String {
    if unchanged.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "\n<details>\n<summary>{} baselined function(s) unchanged</summary>\n\n\
        As recorded in the baseline; the score at or over its limit is in bold.\n\n\
        | Function | Complexity | CRAP |\n|---|---|---|\n",
        unchanged.len()
    );
    for r in unchanged.iter().take(max_rows) {
        let f = r.function;
        let crap = f.crap.map_or_else(|| "n/a".to_string(), kept_crap_text);
        out += &format!(
            "| {} | {} | {} |\n",
            function_cell(f),
            bold_if(f.complexity.to_string(), r.complexity),
            bold_if(crap, r.crap),
        );
    }
    if unchanged.len() > max_rows {
        out += &format!("\n_{} more not listed._\n", unchanged.len() - max_rows);
    }
    out + "\n</details>\n"
}

fn bold_if(text: String, over: bool) -> String {
    if over { format!("**{text}**") } else { text }
}

/// The summary of a failing gate.
pub(crate) fn failing(
    failing: &[Risk<'_>],
    limits: &Thresholds,
    baseline: Option<&Baseline>,
    max_rows: usize,
    moves: &[Movement<'_>],
    unchanged: &[Risk<'_>],
) -> String {
    let failures: Vec<Failure<'_>> = failing.iter().map(|r| Failure::new(r, limits)).collect();

    let mut out = format!(
        "{MARKER}\n### jci-coverage gate: fails\n\n{}\n\n",
        headline(&failures, limits)
    );
    out += if baseline.is_some() {
        "| Function | Over | Baseline | Fix |\n|---|---|---|---|\n"
    } else {
        "| Function | Over | Fix |\n|---|---|---|\n"
    };
    for r in failures.iter().take(max_rows) {
        let mut cells = vec![function_cell(r.risk.function), over_label(r)];
        if let Some(b) = baseline {
            cells.push(against_baseline(r, b));
        }
        cells.push(fix(r, limits));
        out += &format!("| {} |\n", cells.join(" | "));
    }
    if failures.len() > max_rows {
        let more = failures.len() - max_rows;
        out += &format!("\n_{more} more not shown; see the full report in the CI log._\n");
    }
    out += &moved_section(moves, max_rows);
    out += &unchanged_section(unchanged, max_rows);
    out += &format!("\n{}\n", resolution(limits));
    out
}

/// The summary of a passing gate: `allowed` is how many functions are over a
/// limit but allowed by the baseline, when one is in use.
pub(crate) fn passing(
    limits: &Thresholds,
    allowed: Option<usize>,
    max_rows: usize,
    moves: &[Movement<'_>],
    unchanged: &[Risk<'_>],
) -> String {
    let mut on = Vec::new();
    if limits.complexity > 0 {
        on.push(format!("complexity {}", limits.complexity));
    }
    if limits.crap > 0.0 {
        on.push(format!("CRAP {}", limits.crap));
    }
    let on = on.join(", ");
    let body = match allowed.filter(|n| *n > 0) {
        Some(n) => format!(
            "No function fails the limits ({on}); {n} function(s) over a limit are allowed by the baseline."
        ),
        None => format!("No function is at or over the limits ({on})."),
    };
    format!(
        "{MARKER}\n### jci-coverage gate: passes\n\n{body}\n{}{}",
        moved_section(moves, max_rows),
        unchanged_section(unchanged, max_rows)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::complexity::{
        counter::Rules,
        metrics::{FunctionMetrics, crap},
    };

    fn func(
        file: &str,
        name: &str,
        complexity: u32,
        lines: usize,
        covered: usize,
    ) -> FunctionMetrics {
        FunctionMetrics {
            file: file.into(),
            name: name.into(),
            line: 10,
            complexity,
            lines,
            covered,
            crap: (lines > 0).then(|| crap(complexity, covered, lines)),
            is_test: false,
        }
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

    fn rows(out: &str) -> Vec<&str> {
        out.lines().filter(|l| l.starts_with("| `")).collect()
    }

    #[test]
    fn every_summary_starts_with_the_marker_on_its_own_line() {
        let f = func("src/a.rs", "f", 20, 10, 10);
        let fail = failing(&[risk(&f, true, false)], &LIMITS, None, 25, &[], &[]);
        assert_eq!(fail.lines().next(), Some(MARKER), "{fail}");
        let pass = passing(&LIMITS, None, 25, &[], &[]);
        assert_eq!(pass.lines().next(), Some(MARKER), "{pass}");
        assert_eq!(MARKER, "<!-- jci-coverage:gate -->");
    }

    #[test]
    fn the_summary_has_the_headline_and_a_table_with_a_row_per_function() {
        let a = func("crates/x/src/a.rs", "rewrite", 39, 75, 71);
        let b = func("crates/x/src/b.rs", "post_multipart", 5, 26, 0);
        let risks = [risk(&a, true, true), risk(&b, false, true)];
        let out = failing(&risks, &LIMITS, None, 25, &[], &[]);
        assert!(out.contains("### jci-coverage gate: fails"), "{out}");
        assert!(
            out.contains(
                "2 function(s) fail the gate: 1 over the complexity limit (15), 2 over the CRAP limit (30); 1 of the CRAP failures can be fixed with tests alone."
            ),
            "{out}"
        );
        assert!(out.contains("| Function | Over | Fix |"), "{out}");
        assert!(out.contains("|---|---|---|"), "{out}");
        assert_eq!(rows(&out).len(), 2, "{out}");
    }

    #[test]
    fn each_row_gives_the_function_where_it_is_over_and_the_fix() {
        let a = func("crates/x/src/a.rs", "rewrite", 39, 75, 71);
        let b = func("crates/x/src/b.rs", "post_multipart", 5, 26, 0);
        let c = func("src/c.rs", "plain", 20, 10, 10);
        let risks = [
            risk(&a, true, true),
            risk(&b, false, true),
            risk(&c, true, false),
        ];
        let out = failing(&risks, &LIMITS, None, 25, &[], &[]);
        let rows = rows(&out);
        assert_eq!(
            rows[0],
            "| `rewrite`<br>`crates/x/src/a.rs:10` | complexity, CRAP (complexity) | reduce complexity below 15 |"
        );
        assert_eq!(
            rows[1],
            "| `post_multipart`<br>`crates/x/src/b.rs:10` | CRAP (coverage) | cover 1 more line (1 of 26) |"
        );
        assert_eq!(
            rows[2],
            "| `plain`<br>`src/c.rs:10` | complexity | reduce complexity below 15 |"
        );
    }

    #[test]
    fn rows_beyond_the_cap_are_counted_not_listed() {
        let all: Vec<FunctionMetrics> = (0..30)
            .map(|n| func("src/a.rs", &format!("f{n:02}"), 16 + n, 10, 10))
            .collect();
        let risks: Vec<Risk<'_>> = all.iter().map(|f| risk(f, true, false)).collect();
        let out = failing(&risks, &LIMITS, None, 25, &[], &[]);
        assert_eq!(rows(&out).len(), 25, "{out}");
        assert!(
            out.contains("`f24`") && !out.contains("`f25`"),
            "the first 25 in order:\n{out}"
        );
        assert!(
            out.contains("30 function(s) fail the gate"),
            "the headline counts all:\n{out}"
        );
        assert!(
            out.contains("_5 more not shown; see the full report in the CI log._"),
            "{out}"
        );
    }

    #[test]
    fn exactly_the_cap_lists_everything_with_no_note() {
        let all: Vec<FunctionMetrics> = (0..25)
            .map(|n| func("src/a.rs", &format!("f{n:02}"), 16 + n, 10, 10))
            .collect();
        let risks: Vec<Risk<'_>> = all.iter().map(|f| risk(f, true, false)).collect();
        let out = failing(&risks, &LIMITS, None, 25, &[], &[]);
        assert_eq!(rows(&out).len(), 25);
        assert!(!out.contains("not shown"), "{out}");
    }

    #[test]
    fn names_and_paths_are_escaped_for_a_markdown_table() {
        let closure = func("src/a.rs", "", 20, 10, 10);
        let pipe = func("src/we|rd.rs", "a|b", 20, 10, 10);
        let tick = func("src/`x`.rs", "t`i`ck", 20, 10, 10);
        let lines = func("src/a\nb.rs", "multi\nline", 20, 10, 10);
        let slash = func("src\\win\\x.rs", "tail\\|pipe", 20, 10, 10);
        let risks = [
            risk(&closure, true, false),
            risk(&pipe, true, false),
            risk(&tick, true, false),
            risk(&lines, true, false),
            risk(&slash, true, false),
        ];
        let out = failing(&risks, &LIMITS, None, 25, &[], &[]);
        let rows = rows(&out);
        assert_eq!(rows.len(), 5, "every row is one line:\n{out}");
        assert!(rows[0].starts_with("| `<closure>`<br>"), "{}", rows[0]);
        assert!(
            rows[1].contains("`a\\|b`") && rows[1].contains("`src/we\\|rd.rs:10`"),
            "{}",
            rows[1]
        );
        assert!(
            !rows[2].contains("t`i`ck") && rows[2].contains("`t'i'ck`"),
            "{}",
            rows[2]
        );
        assert!(
            rows[3].contains("`multi line`") && rows[3].contains("`src/a b.rs:10`"),
            "{}",
            rows[3]
        );
        // A backslash is shown as `/`, so it cannot meet the `\|` escape.
        assert!(
            rows[4].contains("`tail/\\|pipe`") && rows[4].contains("`src/win/x.rs:10`"),
            "{}",
            rows[4]
        );
    }

    #[test]
    fn cells_are_clipped_so_the_comment_stays_small() {
        let long = "x".repeat(10_000);
        let all: Vec<FunctionMetrics> = (0..25)
            .map(|n| func(&format!("{long}{n}.rs"), &format!("{long}{n}"), 20, 10, 10))
            .collect();
        let risks: Vec<Risk<'_>> = all.iter().map(|f| risk(f, true, false)).collect();
        let out = failing(&risks, &LIMITS, None, 25, &[], &[]);
        assert!(
            out.chars().count() < 20_000,
            "{} characters",
            out.chars().count()
        );
        assert!(out.contains('…'), "clipped cells are marked");
    }

    #[test]
    fn the_unchanged_list_keeps_the_whole_comment_small() {
        let long = "x".repeat(10_000);
        let all: Vec<FunctionMetrics> = (0..60)
            .map(|n| func(&format!("{long}{n}.rs"), &format!("{long}{n}"), 20, 10, 10))
            .collect();
        let risks: Vec<Risk<'_>> = all.iter().map(|f| risk(f, true, true)).collect();
        let out = failing(&risks[..30], &LIMITS, None, 25, &[], &risks[30..]);
        assert!(
            out.chars().count() < 30_000,
            "{} characters",
            out.chars().count()
        );
        assert!(out.contains("_5 more not listed._"), "{out}");
    }

    #[test]
    fn the_summary_never_carries_a_link() {
        // The log may not be readable by everyone who can read the pull request,
        // so a link to it would only disclose where it is.
        let f = func("src/a.rs", "f", 20, 10, 10);
        let fail = failing(&[risk(&f, true, true)], &LIMITS, None, 25, &[], &[]);
        let pass = passing(&LIMITS, Some(3), 25, &[], &[]);
        for out in [&fail, &pass] {
            assert!(!out.contains("http") && !out.contains("]("), "{out}");
        }
    }

    #[test]
    fn the_resolution_names_the_options_and_only_those_that_apply() {
        let f = func("src/a.rs", "f", 20, 10, 10);
        let both = failing(&[risk(&f, true, true)], &LIMITS, None, 25, &[], &[]);
        let tail = both
            .split("**To resolve:**")
            .nth(1)
            .expect("a resolution line");
        for word in [
            "complexity",
            "tests",
            "baseline",
            "--write-baseline",
            "limits",
        ] {
            assert!(tail.contains(word), "{word}: {tail}");
        }
        assert!(
            tail.contains("The CI log has the full report and the exact command."),
            "{tail}"
        );
        let only = failing(
            &[risk(&f, true, false)],
            &limits(15, 0.0),
            None,
            25,
            &[],
            &[],
        );
        let tail = only
            .split("**To resolve:**")
            .nth(1)
            .expect("a resolution line");
        assert!(
            !tail.contains("tests"),
            "complexity-only: tests do not help\n{tail}"
        );
        assert!(!only.contains("CRAP limit"), "{only}");
    }

    #[test]
    fn with_a_baseline_the_table_says_where_each_function_stands() {
        let before = func("src/a.rs", "rewrite", 29, 40, 40);
        let worse = func("src/a.rs", "rewrite", 31, 40, 40);
        let fresh = func("src/a.rs", "fresh", 20, 40, 40);
        let baseline = Baseline::from_functions([&before], Rules::Rca);
        let risks = [risk(&worse, true, false), risk(&fresh, true, false)];
        let out = failing(&risks, &LIMITS, Some(&baseline), 25, &[], &[]);
        assert!(
            out.contains("| Function | Over | Baseline | Fix |"),
            "{out}"
        );
        assert!(out.contains("|---|---|---|---|"), "{out}");
        assert!(
            out.contains("worse than the baseline: complexity 31 > 29"),
            "{out}"
        );
        assert!(out.contains("not in the baseline"), "{out}");
        let without = failing(&risks, &LIMITS, None, 25, &[], &[]);
        assert!(!without.contains("Baseline"), "{without}");
    }

    #[test]
    fn the_baseline_cell_names_only_the_score_the_function_is_over() {
        // Over only the complexity limit: complexity 20 to 21, and its CRAP
        // moved too but is under the CRAP limit.
        let before = func("src/a.rs", "slow", 20, 51, 48);
        let more_complex = func("src/a.rs", "slow", 21, 51, 47);
        // Over only the CRAP limit: uncovered, complexity 5 to 6.
        let cold_before = func("src/a.rs", "cold", 5, 26, 0);
        let cold_now = func("src/a.rs", "cold", 6, 26, 0);
        let baseline = Baseline::from_functions([&before, &cold_before], Rules::Rca);
        let risks = [
            risk(&more_complex, true, false),
            risk(&cold_now, false, true),
        ];
        let out = failing(&risks, &LIMITS, Some(&baseline), 25, &[], &[]);
        let rows = rows(&out);
        assert!(
            rows[0].contains("worse than the baseline: complexity 21 > 20 |"),
            "{}",
            rows[0]
        );
        assert!(!rows[0].contains("CRAP 2"), "{}", rows[0]);
        assert!(
            rows[1].contains("worse than the baseline: CRAP 42.0 > 30.0 |"),
            "{}",
            rows[1]
        );
        assert!(!rows[1].contains("complexity 6"), "{}", rows[1]);
    }

    fn moves_of<'a>(before: &[FunctionMetrics], now: &'a [FunctionMetrics]) -> Vec<Movement<'a>> {
        Baseline::from_functions(before, Rules::Rca).movements(now, &LIMITS, &[])
    }

    #[test]
    fn a_passing_summary_lists_the_baselined_functions_that_moved() {
        let before = [
            func("src/a.rs", "parse", 29, 51, 48),
            func("src/a.rs", "decode", 16, 30, 30),
        ];
        let now = [
            func("src/a.rs", "parse", 29, 51, 47), // CRAP 29.2 to 29.405, shown as 29.5
            func("src/a.rs", "decode", 12, 30, 30), // complexity 16 to 12
        ];
        let moves = moves_of(&before, &now);
        let out = passing(&LIMITS, Some(2), 25, &moves, &[]);
        assert!(out.starts_with(MARKER), "{out}");
        assert!(out.contains("### jci-coverage gate: passes"), "{out}");
        assert!(out.contains("#### Baselined functions that moved"), "{out}");
        assert!(out.contains("1 degraded, 1 improved"), "{out}");
        assert!(
            out.contains("| `parse`<br>`src/a.rs:10` | 29 (same) | 29.2 → 29.5 (worse) |"),
            "{out}"
        );
        assert!(
            out.contains("| `decode`<br>`src/a.rs:10` | 16 → 12 (better) | 16.0 → 12.0 (better) |"),
            "{out}"
        );
        assert!(
            out.find("`parse`") < out.find("`decode`"),
            "degraded first:\n{out}"
        );
        assert!(
            out.contains("--write-baseline"),
            "how to lock in a gain:\n{out}"
        );
    }

    #[test]
    fn nothing_moved_means_no_section() {
        let out = passing(&LIMITS, Some(2), 25, &[], &[]);
        assert!(!out.contains("moved"), "{out}");
        let f = func("src/a.rs", "f", 20, 10, 10);
        let out = failing(&[risk(&f, true, false)], &LIMITS, None, 25, &[], &[]);
        assert!(!out.contains("moved"), "{out}");
    }

    #[test]
    fn a_failing_summary_lists_the_functions_that_moved_before_the_resolution() {
        let before = [func("src/a.rs", "parse", 29, 51, 48)];
        let now = [func("src/a.rs", "parse", 29, 51, 47)];
        let moves = moves_of(&before, &now);
        let bad = func("src/a.rs", "bad", 40, 10, 10);
        let out = failing(&[risk(&bad, true, false)], &LIMITS, None, 25, &moves, &[]);
        let moved = out
            .find("Baselined functions that moved")
            .expect("a section");
        let resolve = out.find("**To resolve:**").expect("the resolution");
        let table = out.find("| `bad`").expect("the failure table");
        assert!(table < moved && moved < resolve, "{out}");
    }

    #[test]
    fn the_moved_table_is_capped_and_says_how_many_are_left_out() {
        let before: Vec<FunctionMetrics> = (0..30)
            .map(|n| func("src/a.rs", &format!("f{n:02}"), 20, 10, 10))
            .collect();
        let now: Vec<FunctionMetrics> = (0..30)
            .map(|n| func("src/a.rs", &format!("f{n:02}"), 21, 10, 10))
            .collect();
        let moves = moves_of(&before, &now);
        let out = passing(&LIMITS, Some(30), 25, &moves, &[]);
        assert_eq!(rows(&out).len(), 25, "{out}");
        assert!(out.contains("30 degraded"), "the count is of all: {out}");
        assert!(out.contains("_5 more not listed._"), "{out}");
        assert!(
            !out.contains("full report"),
            "the log lists only ten moved functions, so it is not the full list: {out}"
        );
    }

    #[test]
    fn a_crap_with_no_value_to_compare_is_shown_plainly() {
        let before = [func("src/a.rs", "g", 20, 10, 10)];
        let mut unknown = func("src/a.rs", "g", 21, 10, 10);
        unknown.crap = None;
        let now = [unknown];
        let moves = moves_of(&before, &now);
        let out = passing(&LIMITS, None, 25, &moves, &[]);
        assert!(out.contains("20 → 21 (worse) | n/a |"), "{out}");
    }

    #[test]
    fn a_passing_gate_says_so_with_the_limits_that_were_on() {
        let out = passing(&LIMITS, None, 25, &[], &[]);
        assert!(out.contains("### jci-coverage gate: passes"), "{out}");
        assert!(
            out.contains("No function is at or over the limits (complexity 15, CRAP 30)."),
            "{out}"
        );
        assert!(!out.contains("baseline"), "{out}");

        let complexity_only = passing(&limits(15, 0.0), None, 25, &[], &[]);
        assert!(
            complexity_only.contains("(complexity 15)."),
            "{complexity_only}"
        );
        assert!(!complexity_only.contains("CRAP"), "{complexity_only}");
    }

    #[test]
    fn a_pass_with_a_baseline_does_not_contradict_itself() {
        // Functions over a limit exist (the baseline allows them), so the first
        // sentence must not claim that none is.
        let out = passing(&LIMITS, Some(6), 25, &[], &[]);
        assert!(
            out.contains(
                "No function fails the limits (complexity 15, CRAP 30); 6 function(s) over a limit are allowed by the baseline."
            ),
            "{out}"
        );
        assert!(!out.contains("at or over the limits"), "{out}");
        let none_allowed = passing(&LIMITS, Some(0), 25, &[], &[]);
        assert!(
            none_allowed.contains("No function is at or over the limits"),
            "{none_allowed}"
        );
    }

    #[test]
    fn unchanged_baselined_functions_are_listed_in_a_collapsed_table() {
        let a = func("crates/x/src/a.rs", "rewrite", 39, 75, 71); // CRAP 39.23, kept as 39.3
        let b = func("crates/x/src/b.rs", "post_multipart", 5, 26, 0); // CRAP 30.0
        let unchanged = [risk(&a, true, true), risk(&b, false, true)];
        let out = passing(&LIMITS, Some(2), 25, &[], &unchanged);
        assert!(
            out.contains("<details>\n<summary>2 baselined function(s) unchanged</summary>\n\n"),
            "a blank line after the summary, or the table is not rendered: {out}"
        );
        assert!(out.trim_end().ends_with("</details>"), "{out}");
        assert!(out.contains("| Function | Complexity | CRAP |"), "{out}");
        assert_eq!(rows(&out).len(), 2, "{out}");
        assert!(
            out.contains("| `rewrite`<br>`crates/x/src/a.rs:10` | **39** | **39.3** |"),
            "over both limits: both bold: {out}"
        );
        assert!(
            out.contains("| `post_multipart`<br>`crates/x/src/b.rs:10` | 5 | **30.0** |"),
            "over the CRAP limit only: only the CRAP is bold: {out}"
        );
        let complexity_only = func("src/c.rs", "parse", 29, 51, 48);
        let out = passing(
            &LIMITS,
            Some(1),
            25,
            &[],
            &[risk(&complexity_only, true, false)],
        );
        assert!(out.contains("| **29** | 29.2 |"), "{out}");
    }

    #[test]
    fn the_unchanged_list_is_in_the_failing_summary_before_the_resolution() {
        let bad = func("src/a.rs", "bad", 20, 10, 10);
        let ok = func("src/b.rs", "ok", 29, 51, 48);
        let out = failing(
            &[risk(&bad, true, false)],
            &LIMITS,
            None,
            25,
            &[],
            &[risk(&ok, true, false)],
        );
        let list = out
            .find("<summary>1 baselined function(s) unchanged")
            .expect(&out);
        assert!(list > out.find("| `bad`").expect(&out), "{out}");
        assert!(list < out.find("**To resolve:**").expect(&out), "{out}");
    }

    #[test]
    fn there_is_no_unchanged_section_when_nothing_is_unchanged() {
        let pass = passing(&LIMITS, Some(3), 25, &[], &[]);
        assert!(!pass.contains("<details>"), "{pass}");
        let f = func("src/a.rs", "f", 20, 10, 10);
        let fail = failing(&[risk(&f, true, false)], &LIMITS, None, 25, &[], &[]);
        assert!(!fail.contains("<details>"), "{fail}");
    }

    #[test]
    fn the_unchanged_list_does_not_change_the_headline() {
        let f = func("src/a.rs", "f", 29, 51, 48);
        let with = passing(&LIMITS, Some(1), 25, &[], &[risk(&f, true, false)]);
        let without = passing(&LIMITS, Some(1), 25, &[], &[]);
        let headline = "No function fails the limits (complexity 15, CRAP 30); 1 function(s) over a limit are allowed by the baseline.";
        assert!(with.contains(headline) && without.contains(headline));
    }

    #[test]
    fn the_unchanged_list_is_capped_and_says_how_many_are_left() {
        let funcs: Vec<_> = (0..5)
            .map(|i| func("src/a.rs", &format!("f{i}"), 29, 51, 48))
            .collect();
        let risks: Vec<_> = funcs.iter().map(|f| risk(f, true, false)).collect();
        let out = passing(&LIMITS, Some(5), 3, &[], &risks);
        assert!(out.contains("5 baselined function(s) unchanged"), "{out}");
        assert_eq!(rows(&out).len(), 3, "{out}");
        assert!(out.contains("_2 more not listed._"), "{out}");
        let all = passing(&LIMITS, Some(5), 5, &[], &risks);
        assert!(!all.contains("more not listed"), "{all}");
    }

    #[test]
    fn an_unchanged_function_with_unknown_coverage_shows_no_crap() {
        let mut f = func("src/a.rs", "f", 29, 0, 0);
        f.crap = None;
        let out = passing(&LIMITS, Some(1), 25, &[], &[risk(&f, true, false)]);
        assert!(out.contains("| **29** | n/a |"), "{out}");
    }

    #[test]
    fn an_unchanged_crap_is_shown_as_the_baseline_keeps_it() {
        // CRAP 29.205 is kept, and compared, as 29.3.
        let mut f = func("src/a.rs", "f", 29, 51, 48);
        f.crap = Some(29.205);
        let out = passing(&LIMITS, Some(1), 25, &[], &[risk(&f, true, false)]);
        assert!(out.contains("| **29** | 29.3 |"), "{out}");
    }
}
