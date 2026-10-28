//! The gate failure as a short markdown summary, for a PR comment or a file.
//!
//! The log report ([`super::render`]) is written for a terminal: wide, with the
//! full resolution text. This is the same analysis shortened for a comment that
//! people read on the pull request: the headline counts, one compact row per
//! failing function (capped, so the comment stays small), the resolution options
//! in a line, and a pointer to the CI log that has the full report.
//!
//! The first line is a hidden marker, so a later run can find the comment it
//! wrote and edit it in place. Names and paths come from the repository, so they
//! are put in code spans, with `|` escaped for the table, and clipped. There is no
//! link to the CI job: its log may not be readable by everyone who can read the
//! pull request, so a link would only disclose where it is.

use super::{Failure, against_baseline, display_name, fix, headline, over_label};
use crate::complexity::{
    baseline::Baseline,
    metrics::{Risk, Thresholds},
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
fn function_cell(r: &Failure<'_>) -> String {
    let f = r.function;
    let name = code(&clip(&display_name(f), MAX_CELL));
    let place = code(&format!("{}:{}", clip(&f.file, MAX_CELL), f.line));
    format!("{name}<br>{place}")
}

/// The one-line "To resolve" pointer: the options that apply to the limits set.
fn resolution(limits: &Thresholds) -> String {
    let mut text = String::from("**To resolve:** reduce the complexity of each function listed");
    if limits.crap > 0.0 {
        text += ", add tests for its own lines (the Fix column says how many are enough)";
    }
    text += ", accept today's state with a baseline (`jci-coverage complexity --write-baseline`, \
            then commit `.jci-coverage-baseline`), or change the limits. \
            The CI log has the full report and the exact command.";
    text
}

/// The summary of a failing gate.
pub(crate) fn failing(
    failing: &[Risk<'_>],
    limits: &Thresholds,
    baseline: Option<&Baseline>,
    max_rows: usize,
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
        let mut cells = vec![function_cell(r), over_label(r)];
        if let Some(b) = baseline {
            cells.push(against_baseline(r.function, b));
        }
        cells.push(fix(r, limits));
        out += &format!("| {} |\n", cells.join(" | "));
    }
    if failures.len() > max_rows {
        let more = failures.len() - max_rows;
        out += &format!("\n_{more} more not shown; see the full report in the CI log._\n");
    }
    out += &format!("\n{}\n", resolution(limits));
    out
}

/// The summary of a passing gate: `allowed` is how many functions are over a
/// limit but allowed by the baseline, when one is in use.
pub(crate) fn passing(limits: &Thresholds, allowed: Option<usize>) -> String {
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
    format!("{MARKER}\n### jci-coverage gate: passes\n\n{body}\n")
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
        let fail = failing(&[risk(&f, true, false)], &LIMITS, None, 25);
        assert_eq!(fail.lines().next(), Some(MARKER), "{fail}");
        let pass = passing(&LIMITS, None);
        assert_eq!(pass.lines().next(), Some(MARKER), "{pass}");
        assert_eq!(MARKER, "<!-- jci-coverage:gate -->");
    }

    #[test]
    fn the_summary_has_the_headline_and_a_table_with_a_row_per_function() {
        let a = func("crates/x/src/a.rs", "rewrite", 39, 75, 71);
        let b = func("crates/x/src/b.rs", "post_multipart", 5, 26, 0);
        let risks = [risk(&a, true, true), risk(&b, false, true)];
        let out = failing(&risks, &LIMITS, None, 25);
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
        let out = failing(&risks, &LIMITS, None, 25);
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
        let out = failing(&risks, &LIMITS, None, 25);
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
        let out = failing(&risks, &LIMITS, None, 25);
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
        let out = failing(&risks, &LIMITS, None, 25);
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
        let out = failing(&risks, &LIMITS, None, 25);
        assert!(
            out.chars().count() < 20_000,
            "{} characters",
            out.chars().count()
        );
        assert!(out.contains('…'), "clipped cells are marked");
    }

    #[test]
    fn the_summary_never_carries_a_link() {
        // The log may not be readable by everyone who can read the pull request,
        // so a link to it would only disclose where it is.
        let f = func("src/a.rs", "f", 20, 10, 10);
        let fail = failing(&[risk(&f, true, true)], &LIMITS, None, 25);
        let pass = passing(&LIMITS, Some(3));
        for out in [&fail, &pass] {
            assert!(!out.contains("http") && !out.contains("]("), "{out}");
        }
    }

    #[test]
    fn the_resolution_names_the_options_and_only_those_that_apply() {
        let f = func("src/a.rs", "f", 20, 10, 10);
        let both = failing(&[risk(&f, true, true)], &LIMITS, None, 25);
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
        let only = failing(&[risk(&f, true, false)], &limits(15, 0.0), None, 25);
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
        let out = failing(&risks, &LIMITS, Some(&baseline), 25);
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
        let without = failing(&risks, &LIMITS, None, 25);
        assert!(!without.contains("Baseline"), "{without}");
    }

    #[test]
    fn a_passing_gate_says_so_with_the_limits_that_were_on() {
        let out = passing(&LIMITS, None);
        assert!(out.contains("### jci-coverage gate: passes"), "{out}");
        assert!(
            out.contains("No function is at or over the limits (complexity 15, CRAP 30)."),
            "{out}"
        );
        assert!(!out.contains("baseline"), "{out}");

        let complexity_only = passing(&limits(15, 0.0), None);
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
        let out = passing(&LIMITS, Some(6));
        assert!(
            out.contains(
                "No function fails the limits (complexity 15, CRAP 30); 6 function(s) over a limit are allowed by the baseline."
            ),
            "{out}"
        );
        assert!(!out.contains("at or over the limits"), "{out}");
        let none_allowed = passing(&LIMITS, Some(0));
        assert!(
            none_allowed.contains("No function is at or over the limits"),
            "{none_allowed}"
        );
    }
}
