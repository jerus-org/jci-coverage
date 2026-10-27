//! The gate failure as a short markdown summary, for a PR comment or a file.
//!
//! The log report ([`super::render`]) is written for a terminal: wide, with the
//! full resolution text. This is the same analysis shortened for a comment that
//! people read on the pull request: the headline counts, one compact row per
//! failing function (capped, so the comment stays small), the resolution options
//! in a line, and a link to the CI job that has the full report.
//!
//! The first line is a hidden marker, so a later run can find the comment it
//! wrote and edit it in place. Names and paths come from the repository, so they
//! are put in code spans, with `|` escaped for the table, and clipped; the job
//! link is used only if it is a plain `https` URL.

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

/// The longest job link that is used.
const MAX_URL: usize = 500;

/// What the summary needs besides the failures.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Context<'a> {
    /// Link to the CI job that has the full report.
    pub(crate) job_url: Option<&'a str>,
    /// How many rows to list.
    pub(crate) max_rows: usize,
}

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
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    format!("`{}`", clean.replace('|', "\\|"))
}

/// The job link, if it is a plain `https` URL that cannot break out of a
/// markdown link or add markup; anything else is left out.
fn safe_url(url: Option<&str>) -> Option<&str> {
    let url = url?;
    let plain = url.len() > "https://".len()
        && url.len() <= MAX_URL
        && url.starts_with("https://")
        && !url.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(
                    c,
                    '<' | '>' | '"' | '`' | '(' | ')' | '[' | ']' | '\\' | '|'
                )
        });
    plain.then_some(url)
}

/// `[CI job](url)` when there is a safe link, else `None`.
fn job_link(context: &Context<'_>) -> Option<String> {
    safe_url(context.job_url).map(|url| format!("[CI job]({url})"))
}

/// The "Function" cell: the name, and below it where it is.
fn function_cell(r: &Failure<'_>) -> String {
    let f = r.function;
    let name = code(&clip(&display_name(f), MAX_CELL));
    let place = code(&format!("{}:{}", clip(&f.file, MAX_CELL), f.line));
    format!("{name}<br>{place}")
}

/// The one-line "To resolve" pointer: the options that apply to the limits set.
fn resolution(limits: &Thresholds, link: Option<&str>) -> String {
    let mut text = String::from("**To resolve:** reduce the complexity of each function listed");
    if limits.crap > 0.0 {
        text += ", add tests for its own lines (the Fix column says how many are enough)";
    }
    text += ", accept today's state with a baseline (`jci-coverage complexity --write-baseline`, \
            then commit `.jci-coverage-baseline`), or change the limits. ";
    match link {
        Some(link) => text += &format!("The {link} has the full report and the exact command."),
        None => text += "The job log has the full report and the exact command.",
    }
    text
}

/// The summary of a failing gate.
pub(crate) fn failing(
    failing: &[Risk<'_>],
    limits: &Thresholds,
    baseline: Option<&Baseline>,
    context: &Context<'_>,
) -> String {
    let failures: Vec<Failure<'_>> = failing.iter().map(|r| Failure::new(r, limits)).collect();
    let link = job_link(context);

    let mut out = format!(
        "{MARKER}\n### jci-coverage gate: fails\n\n{}\n\n",
        headline(&failures, limits)
    );
    out += if baseline.is_some() {
        "| Function | Over | Baseline | Fix |\n|---|---|---|---|\n"
    } else {
        "| Function | Over | Fix |\n|---|---|---|\n"
    };
    for r in failures.iter().take(context.max_rows) {
        let mut cells = vec![function_cell(r), over_label(r)];
        if let Some(b) = baseline {
            cells.push(against_baseline(r.function, b));
        }
        cells.push(fix(r, limits));
        out += &format!("| {} |\n", cells.join(" | "));
    }
    if failures.len() > context.max_rows {
        let more = failures.len() - context.max_rows;
        out += &match &link {
            Some(link) => {
                format!("\n_{more} more not shown; see the full report in the {link}._\n")
            }
            None => format!("\n_{more} more not shown; see the job log._\n"),
        };
    }
    out += &format!("\n{}\n", resolution(limits, link.as_deref()));
    out
}

/// The summary of a passing gate: `allowed` is how many functions are over a
/// limit but allowed by the baseline, when one is in use.
pub(crate) fn passing(
    limits: &Thresholds,
    allowed: Option<usize>,
    context: &Context<'_>,
) -> String {
    let mut on = Vec::new();
    if limits.complexity > 0 {
        on.push(format!("complexity {}", limits.complexity));
    }
    if limits.crap > 0.0 {
        on.push(format!("CRAP {}", limits.crap));
    }
    let mut out = format!(
        "{MARKER}\n### jci-coverage gate: passes\n\nNo function is at or over the limits ({}).",
        on.join(", ")
    );
    if let Some(n) = allowed.filter(|n| *n > 0) {
        out += &format!(" {n} function(s) over a limit are allowed by the baseline.");
    }
    if let Some(link) = job_link(context) {
        out += &format!(" See the {link}.");
    }
    out.push('\n');
    out
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
    const URL: &str = "https://circleci.com/gh/jerus-org/jci-coverage/2064";

    fn ctx(url: Option<&str>, max_rows: usize) -> Context<'_> {
        Context {
            job_url: url,
            max_rows,
        }
    }

    fn rows(out: &str) -> Vec<&str> {
        out.lines().filter(|l| l.starts_with("| `")).collect()
    }

    #[test]
    fn every_summary_starts_with_the_marker_on_its_own_line() {
        let f = func("src/a.rs", "f", 20, 10, 10);
        let fail = failing(&[risk(&f, true, false)], &LIMITS, None, &ctx(None, 25));
        assert_eq!(fail.lines().next(), Some(MARKER), "{fail}");
        let pass = passing(&LIMITS, None, &ctx(None, 25));
        assert_eq!(pass.lines().next(), Some(MARKER), "{pass}");
        assert_eq!(MARKER, "<!-- jci-coverage:gate -->");
    }

    #[test]
    fn the_summary_has_the_headline_and_a_table_with_a_row_per_function() {
        let a = func("crates/x/src/a.rs", "rewrite", 39, 75, 71);
        let b = func("crates/x/src/b.rs", "post_multipart", 5, 26, 0);
        let risks = [risk(&a, true, true), risk(&b, false, true)];
        let out = failing(&risks, &LIMITS, None, &ctx(Some(URL), 25));
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
        let out = failing(&risks, &LIMITS, None, &ctx(None, 25));
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
        let out = failing(&risks, &LIMITS, None, &ctx(Some(URL), 25));
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
            out.contains(&format!(
                "_5 more not shown; see the full report in the [CI job]({URL})._"
            )),
            "{out}"
        );
        let without = failing(&risks, &LIMITS, None, &ctx(None, 25));
        assert!(
            without.contains("_5 more not shown; see the job log._"),
            "{without}"
        );
    }

    #[test]
    fn exactly_the_cap_lists_everything_with_no_note() {
        let all: Vec<FunctionMetrics> = (0..25)
            .map(|n| func("src/a.rs", &format!("f{n:02}"), 16 + n, 10, 10))
            .collect();
        let risks: Vec<Risk<'_>> = all.iter().map(|f| risk(f, true, false)).collect();
        let out = failing(&risks, &LIMITS, None, &ctx(None, 25));
        assert_eq!(rows(&out).len(), 25);
        assert!(!out.contains("not shown"), "{out}");
    }

    #[test]
    fn names_and_paths_are_escaped_for_a_markdown_table() {
        let closure = func("src/a.rs", "", 20, 10, 10);
        let pipe = func("src/we|rd.rs", "a|b", 20, 10, 10);
        let tick = func("src/`x`.rs", "t`i`ck", 20, 10, 10);
        let lines = func("src/a\nb.rs", "multi\nline", 20, 10, 10);
        let risks = [
            risk(&closure, true, false),
            risk(&pipe, true, false),
            risk(&tick, true, false),
            risk(&lines, true, false),
        ];
        let out = failing(&risks, &LIMITS, None, &ctx(None, 25));
        let rows = rows(&out);
        assert_eq!(rows.len(), 4, "every row is one line:\n{out}");
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
    }

    #[test]
    fn cells_are_clipped_so_the_comment_stays_small() {
        let long = "x".repeat(10_000);
        let all: Vec<FunctionMetrics> = (0..25)
            .map(|n| func(&format!("{long}{n}.rs"), &format!("{long}{n}"), 20, 10, 10))
            .collect();
        let risks: Vec<Risk<'_>> = all.iter().map(|f| risk(f, true, false)).collect();
        let out = failing(&risks, &LIMITS, None, &ctx(Some(URL), 25));
        assert!(
            out.chars().count() < 20_000,
            "{} characters",
            out.chars().count()
        );
        assert!(out.contains('…'), "clipped cells are marked");
    }

    #[test]
    fn only_a_plain_https_job_link_is_used() {
        let f = func("src/a.rs", "f", 20, 10, 10);
        let with =
            |url: &str| failing(&[risk(&f, true, false)], &LIMITS, None, &ctx(Some(url), 25));
        assert!(
            with(URL).contains(&format!("[CI job]({URL})")),
            "{}",
            with(URL)
        );
        for bad in [
            "http://circleci.com/gh/a/b/1",
            "javascript:alert(1)",
            "https://x.example/a b",
            "https://x.example/a)[evil](http://e",
            "https://x.example/a>b",
            "https://x.example/\"q",
            // One case per character that could break out of a link or add markup.
            "https://x.example/a(b",
            "https://x.example/a)b",
            "https://x.example/a[b",
            "https://x.example/a]b",
            "https://x.example/a`b",
            "https://x.example/a\\b",
            "https://x.example/a|b",
            "https://x.example/a<b",
            "https://x.example/a\u{7}b",
            "https://",
            "",
        ] {
            let out = with(bad);
            assert!(
                !out.contains("[CI job]"),
                "{bad:?} must not be linked:\n{out}"
            );
            assert!(
                !out.contains(bad) || bad.is_empty(),
                "{bad:?} must not appear:\n{out}"
            );
        }
        let none = failing(&[risk(&f, true, false)], &LIMITS, None, &ctx(None, 25));
        assert!(!none.contains("[CI job]"), "{none}");
    }

    #[test]
    fn the_resolution_names_the_options_and_only_those_that_apply() {
        let f = func("src/a.rs", "f", 20, 10, 10);
        let both = failing(&[risk(&f, true, true)], &LIMITS, None, &ctx(Some(URL), 25));
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
            tail.contains(&format!("[CI job]({URL})")),
            "links to the exact command: {tail}"
        );
        let only = failing(
            &[risk(&f, true, false)],
            &limits(15, 0.0),
            None,
            &ctx(None, 25),
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
        let out = failing(&risks, &LIMITS, Some(&baseline), &ctx(None, 25));
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
        let without = failing(&risks, &LIMITS, None, &ctx(None, 25));
        assert!(!without.contains("Baseline"), "{without}");
    }

    #[test]
    fn a_passing_gate_says_so_with_the_limits_that_were_on() {
        let out = passing(&LIMITS, None, &ctx(Some(URL), 25));
        assert!(out.contains("### jci-coverage gate: passes"), "{out}");
        assert!(
            out.contains("No function is at or over the limits (complexity 15, CRAP 30)."),
            "{out}"
        );
        assert!(out.contains(&format!("[CI job]({URL})")), "{out}");
        assert!(!out.contains("baseline"), "{out}");

        let complexity_only = passing(&limits(15, 0.0), None, &ctx(None, 25));
        assert!(
            complexity_only.contains("(complexity 15)."),
            "{complexity_only}"
        );
        assert!(!complexity_only.contains("CRAP"), "{complexity_only}");

        let baselined = passing(&LIMITS, Some(6), &ctx(None, 25));
        assert!(
            baselined.contains("6 function(s) over a limit are allowed by the baseline."),
            "{baselined}"
        );
    }
}
