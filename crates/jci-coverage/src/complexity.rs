//! Cyclomatic complexity for Rust functions, written into a Cobertura report.
//!
//! `cargo llvm-cov` writes `complexity="0"` everywhere in its Cobertura output,
//! because LLVM coverage data has no complexity. This subcommand computes it
//! from the source and writes it into each `<method>`, which is where OtterWise
//! reads it for its Complexity and CRAP scores.
//!
//! Rust only: the report must be a Cobertura report that `cargo llvm-cov`
//! wrote for Rust source. A class that is not a `.rs` file, or a report without
//! `cargo llvm-cov`'s per-function entries, is refused before anything is
//! read or written.
//!
//! Limitations, from reading source rather than compiled code:
//!
//! - A closure inside a macro's arguments (`assert!(v.iter().any(|x| ..))`) is
//!   reported by `cargo llvm-cov` but invisible to the parser, so it is
//!   unmatched; its decisions count towards the enclosing function instead.
//! - Two functions that start on one source line are told apart by name, and
//!   two closures on one line by their `{closure#N}` index.
//! - Sources are read relative to `--source-root` (default `.`), not the
//!   report's `<sources>`: run from the directory the report was built in, or
//!   pass `--source-root`.
//! - The annotated report replaces `--file` with a regular file, so a symlink
//!   is not followed and the original's mode is not kept. Use `--output` to
//!   write elsewhere.

mod baseline;
mod cobertura;
mod counter;
mod gate_report;
mod metrics;
mod plan;

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::github;

use baseline::{Baseline, Movement};
pub use counter::Rules;
use metrics::{FunctionMetrics, Thresholds};
use plan::{Analyses, Options, Plan, Summary};
pub use plan::{Duplicates, Unmatched};

/// Where `--baseline` and `--write-baseline` read and write without a value: this
/// file in the directory `jci-coverage` runs from, which by convention is the
/// repository root, where the report's workspace-relative file paths start.
pub const DEFAULT_BASELINE: &str = ".jci-coverage-baseline";

/// Parsed `complexity` subcommand arguments.
#[derive(Debug, Args)]
pub struct ComplexityArgs {
    /// Cobertura report to annotate.
    ///
    /// The Cobertura file `report` writes.
    #[arg(long, value_name = "PATH", default_value = "coverage/cobertura.xml")]
    pub file: PathBuf,

    /// Write the annotated report here instead of over `--file`.
    ///
    /// The default replaces the report in place; a failed run never leaves a
    /// half-written file.
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Directory the report's source file paths are relative to.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub source_root: PathBuf,

    /// What to do with the several entries `cargo llvm-cov` writes for one
    /// generic function.
    #[arg(long, value_enum, default_value_t = Duplicates::Merge)]
    pub duplicates: Duplicates,

    /// Which decision points count.
    #[arg(long, value_enum, default_value_t = Rules::Rca)]
    pub rules: Rules,

    /// What to do with a method that has no matching function in the source.
    #[arg(long, value_enum, default_value_t = Unmatched::Keep)]
    pub unmatched: Unmatched,

    /// Leave the class, package and root complexity as they are.
    ///
    /// By default they are set to the sum of the methods below them, so other
    /// Cobertura consumers see consistent data. OtterWise ignores them.
    #[arg(long)]
    pub no_rollup: bool,

    /// Count `#[cfg(test)]` code and `#[test]` functions as complexity 0.
    #[arg(long)]
    pub exclude_tests: bool,

    /// Warn about each function whose cyclomatic complexity is at least this.
    ///
    /// A per-line average can hide a few very risky functions among many simple
    /// ones. 0 turns the check off.
    #[arg(long, value_name = "N", default_value_t = 10)]
    pub warn_complexity: u32,

    /// Warn about each function whose CRAP score is at least this.
    ///
    /// CRAP is complexity squared times the uncovered share cubed, plus
    /// complexity, from the covered share of the function's own lines. 0 turns
    /// the check off.
    #[arg(long, value_name = "N", default_value_t = 30.0, value_parser = threshold)]
    pub warn_crap: f64,

    /// Fail if any function's cyclomatic complexity is at least this.
    ///
    /// The report is written first (not with `--dry-run`, which still fails). 0
    /// turns the check off.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub max_complexity: u32,

    /// Fail if any function's CRAP score is at least this.
    ///
    /// A function with no executable lines has no CRAP score and never fails
    /// this. 0 turns the check off.
    #[arg(long, value_name = "N", default_value_t = 0.0, value_parser = threshold)]
    pub max_crap: f64,

    /// Let the functions listed in this file stay over a maximum.
    ///
    /// A function in the baseline fails only if it has got worse than it was
    /// recorded in a score that is at or over its maximum (a CRAP that moves
    /// while under `--max-crap` does not fail a function accepted for its
    /// complexity). It is still warned about, and one whose scores moved from the
    /// recorded ones is reported (worse or better) without failing. Without a
    /// value it reads
    /// `.jci-coverage-baseline` in the directory this runs from, by convention
    /// the repository root. Make the file with `--write-baseline`; it is a
    /// binary file for this tool, not meant to be edited.
    #[arg(
        long,
        value_name = "PATH",
        num_args = 0..=1,
        default_missing_value = DEFAULT_BASELINE,
        conflicts_with = "write_baseline"
    )]
    pub baseline: Option<PathBuf>,

    /// Record the functions now over a maximum in this file, and do not fail.
    ///
    /// Needs `--max-complexity` or `--max-crap`. Without a value it writes
    /// `.jci-coverage-baseline` in the directory this runs from, by convention
    /// the repository root. Nothing is written with `--dry-run`.
    #[arg(
        long,
        value_name = "PATH",
        num_args = 0..=1,
        default_missing_value = DEFAULT_BASELINE,
        conflicts_with_all = ["summary_file", "pr_comment"]
    )]
    pub write_baseline: Option<PathBuf>,

    /// Write a short markdown summary of the gate result to this file.
    ///
    /// The failing functions (or that the gate passes) in a form for a pull
    /// request comment: the headline, a compact table, and a pointer to the CI
    /// log. Written only when requested, when a maximum is set, after the report;
    /// nothing is written with `--dry-run` or `--write-baseline`. A failure to
    /// write it is a warning and does not change the result.
    #[arg(long, value_name = "PATH")]
    pub summary_file: Option<PathBuf>,

    /// Post the gate summary as a comment on the pull request.
    ///
    /// The same summary as `--summary-file`, posted to the pull request this
    /// build is for (from `CIRCLE_PULL_REQUEST`), using the token in
    /// `GITHUB_TOKEN`. An earlier comment from this tool is edited in place,
    /// otherwise one is created, whether the gate passes or fails, so the pull
    /// request shows that the gate ran. Nothing is posted, and a reason is
    /// logged, when there is no token or no pull request, or the pull request
    /// is from a fork. A failure to post is a warning and does not change the
    /// result.
    #[arg(long)]
    pub pr_comment: bool,

    /// Judge test code against the warning and maximum thresholds too.
    #[arg(long)]
    pub gate_tests: bool,

    /// Only run the maximum and baseline checks, and leave the report as it is.
    ///
    /// For a gate step after `upload`, so a failure comes after the upload and
    /// not before it. The report is not rewritten and the summary and warnings,
    /// which the annotating step has already given, are not repeated. With no
    /// maximum set there is nothing to check and it succeeds.
    #[arg(long, conflicts_with = "output")]
    pub check_only: bool,

    /// Work out and print the summary without writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

/// A CRAP threshold: a finite number, not below 0.
fn threshold(text: &str) -> Result<f64, String> {
    let n: f64 = text.parse().map_err(|e| format!("{e}"))?;
    if n.is_finite() && n >= 0.0 {
        Ok(n)
    } else {
        Err("must be a number of 0 or more".to_string())
    }
}

/// Parse every source file the report mentions. A file that cannot be read or
/// parsed is left out, so its methods are unmatched.
fn analyse_sources(report: &cobertura::Report, root: &Path) -> Analyses {
    let mut analyses = Analyses::new();
    let mut seen = HashSet::new();
    for class in &report.classes {
        if !seen.insert(class.filename.as_str()) {
            continue;
        }
        let path = root.join(&class.filename);
        let analysed = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))
            .and_then(|source| counter::analyze(&source));
        match analysed {
            Ok(entries) => {
                analyses.insert(class.filename.clone(), entries);
            }
            Err(e) => {
                tracing::warn!(file = %path.display(), error = %format!("{e:#}"), "skipping source file");
            }
        }
    }
    analyses
}

/// Replace `path` with `contents`, or leave it as it was: write beside it, then
/// rename over it.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".jci-complexity.tmp");
    let tmp = path.with_file_name(name);
    fs::write(&tmp, contents).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        anyhow::anyhow!("failed to replace {}: {e}", path.display())
    })
}

/// What [`execute`] did, and the per-function scores it measured.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) summary: Summary,
    pub(crate) functions: Vec<FunctionMetrics>,
}

/// Annotate the report, and return what was done.
pub(crate) fn execute(args: &ComplexityArgs) -> Result<Outcome> {
    let xml = fs::read_to_string(&args.file)
        .with_context(|| format!("failed to read {}", args.file.display()))?;
    let report = cobertura::parse(&xml)
        .with_context(|| format!("{} is not a Cobertura report", args.file.display()))?;
    report.require_rust_llvm_cov().with_context(|| {
        format!(
            "{} is not a Rust report from cargo-llvm-cov",
            args.file.display()
        )
    })?;
    let analyses = analyse_sources(&report, &args.source_root);

    tracing::info!(?args, methods = report.methods.len(), "complexity");
    let scored = args.warn_complexity > 0 || args.warn_crap > 0.0 || args.has_maximum();
    let functions = if scored {
        metrics::function_metrics(&report, &analyses, args.rules)
    } else {
        Vec::new()
    };
    if args.check_only {
        return Ok(Outcome {
            summary: Summary::default(),
            functions,
        });
    }
    let options = Options {
        rules: args.rules,
        duplicates: args.duplicates,
        unmatched: args.unmatched,
        rollup: !args.no_rollup,
        exclude_tests: args.exclude_tests,
    };
    let Plan { edit, summary } = plan::plan(&report, &analyses, &options);

    if args.unmatched == Unmatched::Fail && !summary.unmatched.is_empty() {
        let listed: Vec<String> = summary
            .unmatched
            .iter()
            .take(5)
            .map(|m| format!("{} ({}:{})", m.name, m.file, m.line))
            .collect();
        bail!(
            "{} method(s) have no matching function in the source (--unmatched fail): {}{}",
            summary.unmatched.len(),
            listed.join(", "),
            if summary.unmatched.len() > listed.len() {
                ", ..."
            } else {
                ""
            }
        );
    }

    let annotated = cobertura::rewrite(&xml, &edit)?;
    if !args.dry_run {
        write_atomic(
            args.output.as_deref().unwrap_or(&args.file),
            annotated.as_bytes(),
        )?;
    }
    Ok(Outcome { summary, functions })
}

impl ComplexityArgs {
    /// Whether either maximum is on, so there is something to gate.
    fn has_maximum(&self) -> bool {
        self.max_complexity > 0 || self.max_crap > 0.0
    }

    fn thresholds(&self) -> Thresholds {
        Thresholds {
            complexity: self.warn_complexity,
            crap: self.warn_crap,
            include_tests: self.gate_tests,
        }
    }

    fn max_thresholds(&self) -> Thresholds {
        Thresholds {
            complexity: self.max_complexity,
            crap: self.max_crap,
            include_tests: self.gate_tests,
        }
    }
}

/// The risks that are only warnings: a function over a maximum is reported once,
/// as a failure, not also warned about.
fn warnings_only<'a, 'b>(
    risks: &'b [metrics::Risk<'a>],
    over: &[metrics::Risk<'_>],
) -> Vec<&'b metrics::Risk<'a>> {
    risks
        .iter()
        .filter(|r| !over.iter().any(|o| std::ptr::eq(o.function, r.function)))
        .collect()
}

/// A short report of what [`execute`] did.
fn format_summary(s: &Summary) -> String {
    let mean = if s.kept() == 0 {
        0.0
    } else {
        f64::from(s.total) / s.kept() as f64
    };
    let mut out = format!(
        "complexity: {} methods in the report, {} after merging {} duplicate instantiation(s)\n",
        s.methods,
        s.kept(),
        s.merged_away
    );
    out += &format!(
        "  {} matched, {} not matched to a source function\n",
        s.matched,
        s.unmatched.len()
    );
    out += &format!(
        "  total {}, mean {mean:.2}, max {}\n",
        s.total,
        s.max().unwrap_or(0)
    );
    let top: Vec<_> = s.top(5).into_iter().filter(|(c, _)| *c > 0).collect();
    if !top.is_empty() {
        out += "  most complex:\n";
        for (complexity, name) in top {
            out += &format!("    {complexity:>4}  {name}\n");
        }
    }
    out
}

/// A hint for the usual cause of a report where nothing matched.
fn nothing_matched(s: &Summary) -> Option<&'static str> {
    (s.kept() > 0 && s.matched == 0).then_some(
        "no method matched a source function; run from the directory the report was built in, or set --source-root",
    )
}

/// How many risky functions are logged one by one; the rest are counted.
const MAX_RISK_WARNINGS: usize = 10;

/// The summary line for the risky functions, naming only the checks that ran.
fn format_risks(risks: &[metrics::Risk<'_>], args: &ComplexityArgs) -> Option<String> {
    let mut parts = Vec::new();
    if args.warn_complexity > 0 {
        let n = risks.iter().filter(|r| r.complexity).count();
        parts.push(format!(
            "{n} function(s) at or over complexity {}",
            args.warn_complexity
        ));
    }
    if args.warn_crap > 0.0 {
        let n = risks.iter().filter(|r| r.crap).count();
        parts.push(format!("{n} at or over CRAP {}", args.warn_crap));
    }
    (!parts.is_empty()).then(|| format!("  {}", parts.join(", ")))
}

/// The summary line for a baseline: what it let through and what it no longer
/// needs.
fn baseline_summary(accepted: usize, unused: usize) -> Option<String> {
    match (accepted, unused) {
        (0, 0) => None,
        (n, 0) => Some(format!(
            "  {n} function(s) over a maximum are in the baseline"
        )),
        (n, u) => Some(format!(
            "  {n} function(s) over a maximum are in the baseline; {u} baseline entr(ies) no longer needed, regenerate with --write-baseline"
        )),
    }
}

/// Where `path` is, as far as it can be told without creating it: the file
/// itself if it exists, else its directory's real location plus its name.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(real) = fs::canonicalize(path) {
        return real;
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    match (
        fs::canonicalize(parent.unwrap_or_else(|| Path::new("."))),
        path.file_name(),
    ) {
        (Ok(dir), Some(name)) => dir.join(name),
        _ => path.to_path_buf(),
    }
}

/// Whether two paths name the same file, however they are spelled.
fn same_file(a: &Path, b: &Path) -> bool {
    resolved(a) == resolved(b)
}

/// Check the baseline options before anything is read or written, and load the
/// baseline to compare against.
fn check_baseline_args(args: &ComplexityArgs) -> Result<()> {
    if args.write_baseline.is_some() && !args.has_maximum() {
        bail!(
            "--write-baseline records the functions over a maximum: set --max-complexity or --max-crap"
        );
    }
    let report = args.output.as_deref().unwrap_or(&args.file);
    for path in [&args.baseline, &args.write_baseline, &args.summary_file]
        .into_iter()
        .flatten()
    {
        if same_file(path, &args.file) || same_file(path, report) {
            bail!(
                "{} is the same file as the report; give it its own path",
                path.display()
            );
        }
    }
    // Clap already keeps `--baseline` and `--write-baseline` apart; the summary
    // is the one other output that could land on either.
    if let Some(summary) = &args.summary_file {
        for other in [&args.baseline, &args.write_baseline].into_iter().flatten() {
            if same_file(summary, other) {
                bail!(
                    "{} is both the summary and the baseline; give each its own path",
                    summary.display()
                );
            }
        }
    }
    if args.baseline.is_some() && !args.has_maximum() {
        tracing::warn!("--baseline has no effect without --max-complexity or --max-crap");
    }
    if args.pr_comment && !args.has_maximum() {
        tracing::warn!("--pr-comment has no effect without --max-complexity or --max-crap");
    }
    Ok(())
}

/// Read the baseline to compare against, if one was given.
fn load_baseline(args: &ComplexityArgs) -> Result<Option<Baseline>> {
    args.baseline
        .as_deref()
        .map(|path| {
            let bytes =
                fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
            let baseline = Baseline::decode(&bytes)
                .with_context(|| format!("{} cannot be used as a baseline", path.display()))?;
            if baseline.rules() != args.rules {
                bail!(
                    "{} was written with --rules {:?}, not {:?}; scores differ between rule sets, so regenerate it with --write-baseline",
                    path.display(),
                    baseline.rules(),
                    args.rules
                );
            }
            Ok(baseline)
        })
        .transpose()
}

/// The command that records the current state as a baseline, and the file it
/// writes: what the failure report tells the user to run. It repeats the options
/// that decide which functions are over a maximum, so the baseline it makes
/// covers the same functions that failed.
fn write_baseline_record(args: &ComplexityArgs) -> gate_report::Record {
    use clap::ValueEnum;

    let path = args
        .baseline
        .as_ref()
        .or(args.write_baseline.as_ref())
        .map_or_else(|| DEFAULT_BASELINE.to_string(), |p| p.display().to_string());
    let mut words = vec!["jci-coverage complexity".to_string(), "--check-only".into()];
    if args.file != Path::new("coverage/cobertura.xml") {
        words.push(format!("--file {}", args.file.display()));
    }
    if args.source_root != Path::new(".") {
        words.push(format!("--source-root {}", args.source_root.display()));
    }
    if args.rules != Rules::Rca
        && let Some(name) = args.rules.to_possible_value()
    {
        words.push(format!("--rules {}", name.get_name()));
    }
    if args.gate_tests {
        words.push("--gate-tests".into());
    }
    if args.max_complexity > 0 {
        words.push(format!("--max-complexity {}", args.max_complexity));
    }
    if args.max_crap > 0.0 {
        words.push(format!("--max-crap {}", args.max_crap));
    }
    words.push("--write-baseline".into());
    if path != DEFAULT_BASELINE {
        words.push(path.clone());
    }
    gate_report::Record {
        command: words.join(" "),
        path,
    }
}

/// Posts a summary to the pull request, and says
/// what it did.
type Publisher<'a> = &'a dyn Fn(&str);

/// The real thing: the pull request, token and API address come from the
/// environment.
fn publish_to_github(markdown: &str) {
    publish_with_env(|key| std::env::var(key).ok(), markdown);
}

fn publish_with_env(get_env: impl Fn(&str) -> Option<String>, markdown: &str) {
    let api_url = get_env("GITHUB_API_URL").filter(|url| !url.is_empty());
    github::publish(
        &get_env,
        |token| github::ApiClient::new(api_url.as_deref(), token),
        gate_report::MARKER,
        markdown,
    );
}

/// Write and post the markdown summary of the gate result, whichever was asked
/// for, when there is a result to summarise. Failing to do either is a warning:
/// the gate's own result is what matters.
fn publish_summary(
    args: &ComplexityArgs,
    baseline: Option<&Baseline>,
    gate: &Gate<'_>,
    publisher: Publisher<'_>,
) {
    if !args.has_maximum() || (args.summary_file.is_none() && !args.pr_comment) {
        return;
    }
    if args.dry_run {
        if let Some(path) = &args.summary_file {
            println!("(dry run) would write the summary to {}", path.display());
        }
        if args.pr_comment {
            println!("(dry run) would post the summary to the pull request");
        }
        return;
    }
    let limits = args.max_thresholds();
    let passing = gate.failing.is_empty();
    let markdown = if passing {
        let allowed = baseline.map(|_| gate.over.len());
        gate_report::passing_summary(&limits, allowed, gate_report::MAX_ROWS, &gate.moves)
    } else {
        gate_report::failing_summary(
            &gate.failing,
            &limits,
            baseline,
            gate_report::MAX_ROWS,
            &gate.moves,
        )
    };
    if let Some(path) = &args.summary_file {
        match write_atomic(path, markdown.as_bytes()) {
            Ok(()) => println!("wrote the summary to {}", path.display()),
            Err(e) => tracing::warn!(error = %format!("{e:#}"), "could not write the summary"),
        }
    }
    if args.pr_comment {
        publisher(&markdown);
    }
}

/// The functions over a maximum, those of them that fail the gate, and the
/// baselined functions whose scores moved without failing it.
struct Gate<'a> {
    over: Vec<metrics::Risk<'a>>,
    failing: Vec<metrics::Risk<'a>>,
    moves: Vec<Movement<'a>>,
}

fn gate<'a>(
    args: &ComplexityArgs,
    baseline: Option<&Baseline>,
    functions: &'a [FunctionMetrics],
) -> Gate<'a> {
    let over = metrics::risky(functions, &args.max_thresholds());
    // Recording a baseline accepts what is over a maximum; otherwise it is the
    // functions the baseline does not allow that fail.
    let failing = if args.write_baseline.is_some() {
        Vec::new()
    } else {
        over.iter()
            .filter(|r| baseline.is_none_or(|b| !b.allows(r)))
            .copied()
            .collect()
    };
    // Only a baselined function can have moved; the ones failing are reported
    // as failures.
    let moves = baseline.map_or_else(Vec::new, |b| {
        b.movements(functions, args.max_thresholds().include_tests, &failing)
    });
    Gate {
        over,
        failing,
        moves,
    }
}

/// Log the baselined functions whose scores moved: a warning for each that got
/// worse, which is a chance to rework it, and a note for each that got better,
/// which the baseline can be recorded again to hold. Capped like the other
/// warnings.
fn log_moves(moves: &[Movement<'_>]) {
    let shown = |m: &Movement<'_>| {
        let score = |now: Option<String>, then: Option<String>, moved: bool| match (now, then) {
            (Some(now), Some(then)) if moved => format!("{then} -> {now}"),
            (Some(now), _) => now,
            (None, _) => "n/a".to_string(),
        };
        (
            score(
                Some(m.now.complexity.to_string()),
                Some(m.recorded.complexity.to_string()),
                m.complexity.is_some(),
            ),
            score(
                m.now.crap.map(|c| format!("{c:.1}")),
                m.recorded.crap.map(|c| format!("{c:.1}")),
                m.crap.is_some(),
            ),
        )
    };
    for m in moves.iter().take(MAX_RISK_WARNINGS) {
        let f = m.function;
        let (complexity, crap) = shown(m);
        if m.degraded() {
            tracing::warn!(
                file = %f.file, line = f.line, function = %f.name, complexity, crap,
                "baselined function got worse: rework it to bring it back to what was recorded"
            );
        } else {
            tracing::info!(
                file = %f.file, line = f.line, function = %f.name, complexity, crap,
                "baselined function got better: record the baseline again to hold the gain"
            );
        }
    }
    if moves.len() > MAX_RISK_WARNINGS {
        tracing::warn!(
            "{} more baselined function(s) moved, not listed",
            moves.len() - MAX_RISK_WARNINGS
        );
    }
}

/// Report on the baseline, record one if asked, and fail if a function is over
/// a maximum that the baseline does not allow.
fn finish(args: &ComplexityArgs, baseline: Option<&Baseline>, gate: &Gate<'_>) -> Result<()> {
    finish_with(args, baseline, gate, &publish_to_github)
}

fn finish_with(
    args: &ComplexityArgs,
    baseline: Option<&Baseline>,
    gate: &Gate<'_>,
    publisher: Publisher<'_>,
) -> Result<()> {
    let Gate { over, failing, .. } = gate;
    if let Some(b) = baseline {
        let still_over = over.iter().map(|r| r.function);
        if let Some(line) = baseline_summary(over.len() - failing.len(), b.unused(still_over)) {
            println!("{line}");
        }
        log_moves(&gate.moves);
    }
    if let Some(path) = &args.write_baseline {
        let recorded = Baseline::from_functions(over.iter().map(|r| r.function), args.rules);
        if args.dry_run {
            println!(
                "(dry run) would write {} baseline entr(ies) to {}",
                recorded.len(),
                path.display()
            );
        } else {
            write_atomic(path, &recorded.encode())?;
            println!(
                "wrote {} baseline entr(ies) to {}",
                recorded.len(),
                path.display()
            );
        }
        return Ok(());
    }
    publish_summary(args, baseline, gate, publisher);
    if failing.is_empty() {
        return Ok(());
    }
    let record = write_baseline_record(args);
    bail!(
        "{}",
        gate_report::render(failing, &args.max_thresholds(), baseline, &record)
    )
}

/// `--check-only`: run just the maximum and baseline checks.
fn check(
    args: &ComplexityArgs,
    baseline: Option<&Baseline>,
    functions: &[FunctionMetrics],
) -> Result<()> {
    let gate = gate(args, baseline, functions);
    let allowed = baseline.map(|_| gate.over.len() - gate.failing.len());
    println!("{}", check_line(gate.over.len(), allowed));
    finish(args, baseline, &gate)
}

/// The one line `--check-only` prints: what is over a maximum and, with a
/// baseline, how much of that it allows.
fn check_line(over: usize, allowed: Option<usize>) -> String {
    let mut line = format!("complexity check: {over} function(s) over a maximum");
    if let Some(n) = allowed {
        line += &format!(", {n} allowed by the baseline");
    }
    line
}

/// Run the `complexity` subcommand.
pub fn run(args: &ComplexityArgs) -> Result<()> {
    check_baseline_args(args)?;
    if args.check_only && !args.has_maximum() {
        println!("complexity check: no maximum is set, so there is nothing to check");
        return Ok(());
    }
    let baseline = load_baseline(args)?;
    let Outcome { summary, functions } = execute(args)?;
    if args.check_only {
        return check(args, baseline.as_ref(), &functions);
    }
    if let Some(hint) = nothing_matched(&summary) {
        tracing::warn!("{hint}");
    }
    for m in summary.unmatched.iter().take(10) {
        tracing::warn!(file = %m.file, line = m.line, method = %m.name, "no matching function in the source");
    }
    let risks = metrics::risky(&functions, &args.thresholds());
    let verdict = gate(args, baseline.as_ref(), &functions);
    // Everything over a warn threshold or a maximum is warned about, unless it
    // is failing, so accepted and recorded functions stay visible.
    let flagged = metrics::risky(&functions, &args.thresholds().either(args.max_thresholds()));
    let warned = warnings_only(&flagged, &verdict.failing);
    for r in warned.iter().take(MAX_RISK_WARNINGS) {
        let f = r.function;
        match f.crap {
            Some(crap) => tracing::warn!(
                file = %f.file, line = f.line, function = %f.name, complexity = f.complexity,
                crap = format!("{crap:.1}"), covered = f.covered, lines = f.lines,
                "risky function"
            ),
            None => tracing::warn!(
                file = %f.file, line = f.line, function = %f.name, complexity = f.complexity,
                "risky function (no executable lines to measure coverage)"
            ),
        }
    }
    if warned.len() > MAX_RISK_WARNINGS {
        tracing::warn!(
            "{} more risky function(s) not listed",
            warned.len() - MAX_RISK_WARNINGS
        );
    }
    print!("{}", format_summary(&summary));
    if let Some(line) = format_risks(&risks, args) {
        println!("{line}");
    }
    let target = args.output.as_deref().unwrap_or(&args.file);
    if args.dry_run {
        println!("(dry run) would write {}", target.display());
    } else {
        println!("wrote {}", target.display());
    }
    finish(args, baseline.as_ref(), &verdict)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    const SOURCE: &str = "\
fn plain() {}

fn pick(x: Option<i32>) -> Option<i32> {
    let v = x?;
    match v { 0 => None, _ => Some(v) }
}
";

    /// `plain` (1) once, `pick` (4) twice for two instantiations, and one
    /// method the source does not have.
    fn report(root: &str) -> String {
        format!(
            r#"<?xml version="1.0" ?>
<coverage complexity="0" line-rate="1" lines-covered="3" lines-valid="3">
    <sources>
        <source>{root}</source>
    </sources>
    <packages>
        <package name="x" complexity="0">
            <classes>
                <class complexity="0" filename="src/lib.rs" name="x.lib.rs">
                    <methods>
                        <method name="x::plain" signature="" complexity="0" line-rate="1" branch-rate="1">
                            <lines>
                                <line hits="1" number="1" branch="false"/>
                            </lines>
                        </method>
                        <method name="x::pick::&lt;u8&gt;" signature="" complexity="0" line-rate="1" branch-rate="1">
                            <lines>
                                <line hits="2" number="3" branch="false"/>
                            </lines>
                        </method>
                        <method name="x::pick::&lt;u16&gt;" signature="" complexity="0" line-rate="0" branch-rate="0">
                            <lines>
                                <line hits="0" number="3" branch="false"/>
                            </lines>
                        </method>
                        <method name="x::generated" signature="" complexity="0" line-rate="0" branch-rate="0">
                            <lines>
                                <line hits="0" number="99" branch="false"/>
                            </lines>
                        </method>
                    </methods>
                    <lines>
                        <line hits="1" number="1" branch="false"/>
                    </lines>
                </class>
            </classes>
        </package>
    </packages>
</coverage>
"#
        )
    }

    fn setup() -> (tempfile::TempDir, ComplexityArgs) {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("src")).expect("mkdir");
        fs::write(dir.path().join("src/lib.rs"), SOURCE).expect("write source");
        fs::create_dir_all(dir.path().join("coverage")).expect("mkdir");
        let file = dir.path().join("coverage/cobertura.xml");
        fs::write(&file, report("/work")).expect("write report");
        let args = ComplexityArgs {
            file,
            output: None,
            source_root: dir.path().to_path_buf(),
            duplicates: Duplicates::Merge,
            rules: Rules::Rca,
            unmatched: Unmatched::Keep,
            no_rollup: false,
            exclude_tests: false,
            warn_complexity: 10,
            warn_crap: 30.0,
            gate_tests: false,
            max_complexity: 0,
            max_crap: 0.0,
            baseline: None,
            write_baseline: None,
            check_only: false,
            summary_file: None,
            pr_comment: false,
            dry_run: false,
        };
        (dir, args)
    }

    fn complexities(xml: &str) -> Vec<String> {
        xml.lines()
            .filter(|l| l.trim_start().starts_with("<method "))
            .map(|l| {
                let name = l
                    .split("name=\"")
                    .nth(1)
                    .and_then(|r| r.split('"').next())
                    .unwrap();
                let c = l
                    .split(" complexity=\"")
                    .nth(1)
                    .and_then(|r| r.split('"').next())
                    .unwrap();
                format!("{name}={c}")
            })
            .collect()
    }

    #[test]
    fn the_thresholds_come_from_the_arguments() {
        let (_dir, mut args) = setup();
        args.warn_complexity = 4;
        args.warn_crap = 0.0;
        args.gate_tests = true;
        let t = args.thresholds();
        assert_eq!((t.complexity, t.crap, t.include_tests), (4, 0.0, true));

        let outcome = execute(&args).expect("runs");
        let flagged: Vec<_> = metrics::risky(&outcome.functions, &t)
            .iter()
            .map(|r| r.function.name.clone())
            .collect();
        assert_eq!(flagged, vec!["pick"], "{:?}", outcome.functions);
    }

    #[test]
    fn the_risk_summary_names_only_the_checks_that_ran() {
        let (_dir, mut args) = setup();
        let fns = [];
        assert_eq!(
            format_risks(&fns, &args).as_deref(),
            Some("  0 function(s) at or over complexity 10, 0 at or over CRAP 30")
        );
        args.warn_complexity = 0;
        assert_eq!(
            format_risks(&fns, &args).as_deref(),
            Some("  0 at or over CRAP 30")
        );
        args.warn_crap = 0.0;
        assert_eq!(format_risks(&fns, &args), None);
        assert!(execute(&args).expect("runs").functions.is_empty());
    }

    #[test]
    fn a_function_over_a_maximum_fails_the_run_after_the_report_is_written() {
        let (_dir, mut args) = setup();
        args.max_complexity = 4;
        let err = run(&args).unwrap_err().to_string();
        assert!(
            err.contains("pick") && err.contains("Complexity (max 4)"),
            "{err}"
        );
        let xml = fs::read_to_string(&args.file).expect("read");
        assert!(
            xml.contains(r#"complexity="4""#),
            "report still written: {xml}"
        );

        args.max_complexity = 5;
        run(&args).expect("nothing is over 5");
        args.max_complexity = 0;
        run(&args).expect("0 turns the maximum off");
    }

    #[test]
    fn a_crap_maximum_fails_on_a_known_score_and_names_the_limit() {
        let (dir, mut args) = setup();
        // `pick` is complexity 4 with its three own lines uncovered: CRAP 20.
        let xml = fs::read_to_string(&args.file).expect("read");
        let xml = xml.replace(
            "<line hits=\"1\" number=\"1\" branch=\"false\"/>\n                    </lines>\n                </class>",
            "<line hits=\"1\" number=\"1\" branch=\"false\"/>\n<line hits=\"0\" number=\"3\" branch=\"false\"/>\n<line hits=\"0\" number=\"4\" branch=\"false\"/>\n<line hits=\"0\" number=\"5\" branch=\"false\"/>\n                    </lines>\n                </class>",
        );
        fs::write(&args.file, xml).expect("write");
        let _ = &dir;

        args.max_crap = 21.0;
        run(&args).expect("20 is under 21");
        args.max_crap = 20.0;
        let err = run(&args).unwrap_err().to_string();
        assert!(
            err.contains("pick") && err.contains("CRAP (max 20)") && err.contains("20.0"),
            "{err}"
        );
        assert!(err.contains("0/3 (0%)"), "the coverage behind it: {err}");
        assert!(err.contains("cover 1 more line (1 of 3)"), "{err}");
        assert!(
            !err.contains("--max-complexity"),
            "only the limits that are set: {err}"
        );
    }

    #[test]
    fn a_function_over_a_maximum_is_not_also_warned_about() {
        let f = |name: &str, complexity| metrics::FunctionMetrics {
            file: "src/a.rs".into(),
            name: name.into(),
            line: 1,
            complexity,
            lines: 2,
            covered: 2,
            crap: Some(f64::from(complexity)),
            is_test: false,
        };
        let functions = [f("mild", 12), f("severe", 25)];
        let (_dir, mut args) = setup();
        args.max_complexity = 20;
        let risks = metrics::risky(&functions, &args.thresholds());
        let over = metrics::risky(&functions, &args.max_thresholds());
        assert_eq!(risks.len(), 2);
        let warned = warnings_only(&risks, &over);
        let names: Vec<_> = warned.iter().map(|r| r.function.name.as_str()).collect();
        assert_eq!(names, ["mild"]);
    }

    /// Run the subcommand and return what it logged.
    fn logged(args: &ComplexityArgs) -> (Result<()>, String) {
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("lock").extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buf = Buf(Arc::new(Mutex::new(Vec::new())));
        let writer = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        let result = tracing::subscriber::with_default(subscriber, || run(args));
        let text = String::from_utf8(buf.0.lock().expect("lock").clone()).expect("utf8");
        (result, text)
    }

    #[test]
    fn a_risky_function_is_warned_about_when_the_maximum_is_off() {
        let (_dir, mut args) = setup();
        args.warn_complexity = 4;
        assert_eq!(args.max_complexity, 0, "the maximum is off");
        let (result, log) = logged(&args);
        result.expect("warnings never fail the run");
        assert!(
            log.contains("risky function") && log.contains("pick"),
            "{log}"
        );
    }

    #[test]
    fn a_function_over_a_maximum_fails_without_a_warning() {
        let (_dir, mut args) = setup();
        args.warn_complexity = 4;
        args.max_complexity = 4;
        let (result, log) = logged(&args);
        assert!(result.is_err());
        assert!(
            !log.contains("risky function"),
            "reported once, as the failure: {log}"
        );
    }

    #[test]
    fn nothing_is_warned_about_when_both_checks_are_off() {
        let (_dir, mut args) = setup();
        args.warn_complexity = 0;
        args.warn_crap = 0.0;
        let (result, log) = logged(&args);
        result.expect("runs");
        assert!(!log.contains("risky function"), "{log}");
    }

    #[test]
    fn with_the_maximum_off_every_risk_is_only_a_warning() {
        let (_dir, mut args) = setup();
        args.warn_complexity = 4;
        let outcome = execute(&args).expect("runs");
        let risks = metrics::risky(&outcome.functions, &args.thresholds());
        let over = metrics::risky(&outcome.functions, &args.max_thresholds());
        assert!(over.is_empty());
        assert_eq!(warnings_only(&risks, &over).len(), risks.len());
        assert!(!risks.is_empty());
    }

    #[test]
    fn the_suggested_baseline_command_carries_the_options_in_use() {
        let (_dir, mut args) = setup();
        args.file = PathBuf::from("coverage/cobertura.xml");
        args.source_root = PathBuf::from(".");
        args.max_complexity = 15;
        args.max_crap = 30.0;
        let record = write_baseline_record(&args);
        assert_eq!(
            record.command,
            "jci-coverage complexity --check-only --max-complexity 15 --max-crap 30 --write-baseline"
        );
        assert_eq!(record.path, ".jci-coverage-baseline");

        args.file = PathBuf::from("cov.xml");
        args.source_root = PathBuf::from("src");
        args.rules = Rules::Mccabe;
        args.gate_tests = true;
        args.max_crap = 0.0;
        args.baseline = Some(PathBuf::from("ci/b.bin"));
        let record = write_baseline_record(&args);
        assert_eq!(
            record.command,
            "jci-coverage complexity --check-only --file cov.xml --source-root src --rules mccabe --gate-tests --max-complexity 15 --write-baseline ci/b.bin"
        );
        assert_eq!(record.path, "ci/b.bin");
    }

    #[test]
    fn the_failure_names_the_command_for_the_options_the_run_used() {
        let (_dir, mut args) = setup();
        args.max_complexity = 4;
        args.rules = Rules::Rca;
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("--check-only --file"), "{err}");
        assert!(err.contains("--max-complexity 4 --write-baseline"), "{err}");
    }

    /// What `finish_with` made of a gate, with the summaries the publisher was
    /// asked to post: `(result, posted)`.
    fn finish_posting(
        args: &ComplexityArgs,
        baseline: Option<&Baseline>,
    ) -> (Result<()>, Vec<String>) {
        use std::cell::RefCell;

        let functions = execute(args).expect("runs").functions;
        let gate = gate(args, baseline, &functions);
        let asked = RefCell::new(Vec::new());
        let publisher = |markdown: &str| {
            asked.borrow_mut().push(markdown.to_string());
        };
        let result = finish_with(args, baseline, &gate, &publisher);
        (result, asked.into_inner())
    }

    #[test]
    fn pr_comment_posts_the_failure_summary_and_the_gate_still_fails() {
        let (_dir, mut args) = setup();
        args.max_complexity = 4;
        args.pr_comment = true;
        let (result, asked) = finish_posting(&args, None);
        assert!(result.unwrap_err().to_string().contains("pick"));
        assert_eq!(asked.len(), 1);
        let markdown = &asked[0];
        assert_eq!(markdown.lines().next(), Some(gate_report::MARKER));
        assert!(markdown.contains("### jci-coverage gate: fails") && markdown.contains("`pick`"));
    }

    #[test]
    fn pr_comment_posts_the_passing_summary_when_the_gate_passes() {
        let (_dir, mut args) = setup();
        args.max_complexity = 40;
        args.pr_comment = true;
        let (result, asked) = finish_posting(&args, None);
        result.expect("the gate passes");
        assert_eq!(asked.len(), 1);
        assert!(
            asked[0].contains("### jci-coverage gate: passes"),
            "{}",
            asked[0]
        );
    }

    #[test]
    fn nothing_is_posted_unless_asked_or_when_there_is_nothing_to_say() {
        let (dir, mut args) = setup();
        args.max_complexity = 4;
        let (_, asked) = finish_posting(&args, None);
        assert!(asked.is_empty(), "off unless asked for");

        args.pr_comment = true;
        args.max_complexity = 0;
        let (_, asked) = finish_posting(&args, None);
        assert!(asked.is_empty(), "no maximum, nothing to summarise");

        args.max_complexity = 4;
        args.dry_run = true;
        let (_, asked) = finish_posting(&args, None);
        assert!(asked.is_empty(), "a dry run posts nothing");

        args.dry_run = false;
        args.write_baseline = Some(baseline_path(&dir));
        let (_, asked) = finish_posting(&args, None);
        assert!(asked.is_empty(), "recording a baseline is not a verdict");
    }

    #[test]
    fn the_file_and_the_comment_hold_the_same_summary() {
        let (dir, mut args) = setup();
        args.max_complexity = 4;
        args.pr_comment = true;
        let path = dir.path().join("summary.md");
        args.summary_file = Some(path.clone());
        let (_, asked) = finish_posting(&args, None);
        assert_eq!(fs::read_to_string(&path).expect("written"), asked[0]);
    }

    #[test]
    fn the_real_wiring_posts_to_the_api_named_by_the_environment() {
        let (base, server) = github::testing::serve(vec![
            ("200 OK", "[]".to_string()),
            ("201 Created", "{}".to_string()),
        ]);
        let vars = [
            ("GITHUB_TOKEN", "ghs_secret"),
            ("GITHUB_API_URL", base.as_str()),
            (
                "CIRCLE_PULL_REQUEST",
                "https://github.com/jerus-org/jci-coverage/pull/7",
            ),
            ("CIRCLE_PROJECT_USERNAME", "jerus-org"),
            ("CIRCLE_PROJECT_REPONAME", "jci-coverage"),
        ];
        let markdown = format!("{}\nsummary", gate_report::MARKER);
        publish_with_env(
            |key| {
                vars.iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.to_string())
            },
            &markdown,
        );
        let requests = server.join().expect("server");
        assert!(
            requests[0].starts_with("GET /repos/jerus-org/jci-coverage/issues/7/comments?"),
            "{}",
            requests[0]
        );
        assert!(
            requests[1].starts_with("POST /repos/jerus-org/jci-coverage/issues/7/comments "),
            "{}",
            requests[1]
        );
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("authorization: bearer ghs_secret")
                && requests[1].contains("summary"),
            "{}",
            requests[1]
        );
    }

    #[test]
    fn pr_comment_without_a_maximum_is_a_warning() {
        let (_dir, mut args) = setup();
        args.pr_comment = true;
        let (result, log) = logged(&args);
        result.expect("nothing to gate");
        assert!(log.contains("--pr-comment has no effect"), "{log}");
    }

    #[test]
    fn the_summary_file_holds_the_failure_and_the_gate_still_fails() {
        let (dir, mut args) = setup();
        args.max_complexity = 4;
        let path = dir.path().join("summary.md");
        args.summary_file = Some(path.clone());
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("pick"), "the gate still fails: {err}");
        let md = fs::read_to_string(&path).expect("written");
        assert_eq!(md.lines().next(), Some(gate_report::MARKER), "{md}");
        assert!(
            md.contains("### jci-coverage gate: fails") && md.contains("`pick`"),
            "{md}"
        );
    }

    #[test]
    fn the_summary_file_says_so_when_the_gate_passes() {
        let (dir, mut args) = setup();
        args.max_complexity = 5;
        let path = dir.path().join("summary.md");
        args.summary_file = Some(path.clone());
        run(&args).expect("nothing is over 5");
        let md = fs::read_to_string(&path).expect("written");
        assert!(md.contains("### jci-coverage gate: passes"), "{md}");
        assert!(md.contains("(complexity 5)."), "{md}");
    }

    #[test]
    fn the_summary_file_is_written_by_check_only_and_counts_the_baseline() {
        let (dir, mut args) = setup();
        args.check_only = true;
        args.max_complexity = 4;
        let baseline = baseline_path(&dir);
        write_baseline_file(&baseline, "pick", 4, Rules::Rca);
        args.baseline = Some(baseline);
        let path = dir.path().join("summary.md");
        args.summary_file = Some(path.clone());
        run(&args).expect("baselined");
        let md = fs::read_to_string(&path).expect("written");
        assert!(
            md.contains("1 function(s) over a limit are allowed by the baseline."),
            "{md}"
        );
    }

    #[test]
    fn no_summary_is_written_when_there_is_nothing_to_summarise() {
        let (dir, mut args) = setup();
        let path = dir.path().join("summary.md");
        args.summary_file = Some(path.clone());
        run(&args).expect("no maximum is set");
        assert!(!path.exists(), "no maximum, nothing to summarise");

        args.check_only = true;
        run(&args).expect("nothing to check");
        assert!(!path.exists());

        args.check_only = false;
        args.max_complexity = 4;
        args.dry_run = true;
        run(&args).unwrap_err();
        assert!(!path.exists(), "a dry run writes nothing");

        args.dry_run = false;
        args.write_baseline = Some(baseline_path(&dir));
        run(&args).expect("recording a baseline does not fail");
        assert!(!path.exists(), "recording a baseline is not a verdict");
    }

    #[test]
    fn the_summary_file_may_not_be_another_output_of_the_run() {
        let (dir, mut args) = setup();
        args.max_complexity = 4;
        args.summary_file = Some(args.file.clone());
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("same file"), "{err}");

        args.summary_file = Some(baseline_path(&dir));
        args.baseline = args.summary_file.clone();
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("both the summary and the baseline"), "{err}");
    }

    #[test]
    fn paths_are_compared_by_where_they_are_not_how_they_are_spelled() {
        let (dir, mut args) = setup();
        args.max_complexity = 4;
        let report = args.file.clone();
        let name = report.file_name().expect("name").to_owned();
        let parent = report.parent().expect("dir").to_path_buf();

        args.summary_file = Some(
            parent
                .join("..")
                .join(parent.file_name().expect("n"))
                .join(&name),
        );
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("same file"), "{err}");

        args.summary_file = None;
        args.baseline = Some(parent.join(".").join(&name));
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("same file"), "{err}");

        // A file that does not exist yet, spelled two ways.
        args.baseline = None;
        let b = dir.path().join("b.bin");
        args.write_baseline = Some(b.clone());
        args.summary_file = Some(dir.path().join(".").join("b.bin"));
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("same file") || err.contains("both"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_the_report_is_the_report() {
        let (dir, mut args) = setup();
        args.max_complexity = 4;
        let link = dir.path().join("link.md");
        std::os::unix::fs::symlink(&args.file, &link).expect("symlink");
        args.summary_file = Some(link);
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("same file"), "{err}");
    }

    #[test]
    fn a_summary_that_cannot_be_written_is_a_warning_and_the_gate_result_stands() {
        let (dir, mut args) = setup();
        args.max_complexity = 4;
        let blocker = dir.path().join("a-file");
        fs::write(&blocker, "x").expect("write");
        args.summary_file = Some(blocker.join("summary.md"));
        let (result, log) = logged(&args);
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("pick"),
            "the gate's own failure, not the summary's: {err}"
        );
        assert!(log.contains("summary"), "{log}");
    }

    #[test]
    fn the_failure_is_a_table_naming_every_function_with_how_to_resolve_it() {
        let f = |name: &str, complexity| metrics::FunctionMetrics {
            file: "src/a.rs".into(),
            name: name.into(),
            line: 1,
            complexity,
            lines: 2,
            covered: 0,
            crap: Some(f64::from(complexity * complexity + complexity)),
            is_test: false,
        };
        let functions: Vec<_> = (1..=8).map(|n| f(&format!("f{n}"), 10 + n)).collect();
        let (_dir, mut args) = setup();
        args.max_complexity = 10;
        args.max_crap = 100.0;
        let over = metrics::risky(&functions, &args.max_thresholds());
        let gate = Gate {
            failing: over.clone(),
            over,
            moves: Vec::new(),
        };
        let err = finish(&args, None, &gate).unwrap_err().to_string();
        assert!(err.starts_with("8 function(s) fail the gate"), "{err}");
        for n in 1..=8 {
            assert!(err.contains(&format!("| f{n} ")), "f{n} missing: {err}");
        }
        assert!(!err.contains("more)") && !err.contains("... and"), "{err}");
        assert!(err.contains("How to resolve"), "{err}");
    }

    fn baseline_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("baseline.bin")
    }

    /// Write a baseline allowing `name` in `src/lib.rs` up to `complexity`.
    fn write_baseline_file(path: &Path, name: &str, complexity: u32, rules: Rules) {
        let f = metrics::FunctionMetrics {
            file: "src/lib.rs".into(),
            name: name.into(),
            line: 1,
            complexity,
            lines: 0,
            covered: 0,
            crap: None,
            is_test: false,
        };
        fs::write(path, Baseline::from_functions([&f], rules).encode()).expect("write");
    }

    #[test]
    fn a_written_baseline_lets_the_functions_in_it_stay_over_the_maximum() {
        let (dir, mut args) = setup();
        args.warn_complexity = 4;
        args.max_complexity = 4;
        let path = baseline_path(&dir);

        args.write_baseline = Some(path.clone());
        let (result, _) = logged(&args);
        result.expect("writing the baseline accepts the current state");
        let bytes = fs::read(&path).expect("written");
        assert!(bytes.starts_with(b"JCIB"), "a binary baseline: {bytes:?}");
        let names: Vec<_> = fs::read_dir(dir.path())
            .expect("list")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|n| n.ends_with(".tmp")), "{names:?}");

        args.write_baseline = None;
        args.baseline = Some(path);
        let (result, log) = logged(&args);
        result.expect("pick is baselined");
        assert!(
            log.contains("risky function") && log.contains("pick"),
            "still warned about: {log}"
        );
    }

    #[test]
    fn a_baselined_function_is_still_warned_about_with_the_warn_thresholds_off() {
        let (dir, mut args) = setup();
        args.warn_complexity = 0;
        args.warn_crap = 0.0;
        args.max_complexity = 4;
        let path = baseline_path(&dir);
        write_baseline_file(&path, "pick", 4, Rules::Rca);
        args.baseline = Some(path);
        let (result, log) = logged(&args);
        result.expect("allowed by the baseline");
        assert!(
            log.contains("risky function") && log.contains("pick"),
            "{log}"
        );
    }

    #[test]
    fn writing_a_baseline_names_the_functions_it_records() {
        let (dir, mut args) = setup();
        args.warn_complexity = 0;
        args.warn_crap = 0.0;
        args.max_complexity = 4;
        args.write_baseline = Some(baseline_path(&dir));
        let (result, log) = logged(&args);
        result.expect("writes");
        assert!(
            log.contains("risky function") && log.contains("pick"),
            "{log}"
        );
    }

    #[test]
    fn the_baseline_may_not_be_the_report_or_its_output() {
        let (dir, mut args) = setup();
        args.max_complexity = 4;
        args.write_baseline = Some(args.file.clone());
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("same file"), "{err}");

        args.write_baseline = None;
        args.output = Some(dir.path().join("annotated.xml"));
        args.baseline = args.output.clone();
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("same file"), "{err}");
    }

    #[test]
    fn a_baseline_without_a_maximum_is_flagged_as_having_no_effect() {
        let (dir, mut args) = setup();
        let path = baseline_path(&dir);
        write_baseline_file(&path, "pick", 4, Rules::Rca);
        args.baseline = Some(path);
        let (result, log) = logged(&args);
        result.expect("runs");
        assert!(log.contains("--baseline has no effect"), "{log}");
    }

    #[test]
    fn a_function_worse_than_its_baseline_or_missing_from_it_fails() {
        let (dir, mut args) = setup();
        args.max_complexity = 4;
        let path = baseline_path(&dir);
        args.baseline = Some(path.clone());

        write_baseline_file(&path, "pick", 3, Rules::Rca);
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("pick"), "worse than recorded: {err}");

        write_baseline_file(&path, "other", 9, Rules::Rca);
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("pick"), "not in the baseline: {err}");

        write_baseline_file(&path, "pick", 4, Rules::Rca);
        run(&args).expect("as recorded");
    }

    #[test]
    fn a_baseline_that_cannot_be_used_stops_the_run_before_the_report_changes() {
        let (dir, mut args) = setup();
        let before = fs::read_to_string(&args.file).expect("read");
        args.baseline = Some(dir.path().join("missing.bin"));
        let err = format!("{:#}", run(&args).unwrap_err());
        assert!(err.contains("missing.bin"), "{err}");
        assert_eq!(fs::read_to_string(&args.file).expect("read"), before);

        let bad = baseline_path(&dir);
        fs::write(&bad, "not a baseline\n").expect("write");
        args.baseline = Some(bad.clone());
        let err = format!("{:#}", run(&args).unwrap_err());
        assert!(err.contains("cannot be used as a baseline"), "{err}");
        assert_eq!(fs::read_to_string(&args.file).expect("read"), before);

        write_baseline_file(&bad, "pick", 4, Rules::Rca);
        let mut bytes = fs::read(&bad).expect("read");
        let at = bytes.len() / 2;
        bytes[at] ^= 0x04;
        fs::write(&bad, bytes).expect("write");
        let err = format!("{:#}", run(&args).unwrap_err());
        assert!(
            err.contains("corrupt"),
            "a damaged baseline is refused: {err}"
        );
        assert_eq!(fs::read_to_string(&args.file).expect("read"), before);
    }

    #[test]
    fn a_baseline_written_for_the_other_rule_set_is_refused() {
        let (dir, mut args) = setup();
        args.max_complexity = 4;
        let path = baseline_path(&dir);
        write_baseline_file(&path, "pick", 4, Rules::Mccabe);
        args.baseline = Some(path);
        let err = format!("{:#}", run(&args).unwrap_err());
        assert!(err.contains("Mccabe") && err.contains("Rca"), "{err}");
    }

    #[test]
    fn a_baseline_is_written_only_for_a_maximum_that_is_set_and_not_on_a_dry_run() {
        let (dir, mut args) = setup();
        let path = baseline_path(&dir);
        args.write_baseline = Some(path.clone());
        let err = run(&args).unwrap_err().to_string();
        assert!(
            err.contains("--max-complexity") && err.contains("--max-crap"),
            "{err}"
        );
        assert!(!path.exists());

        args.max_complexity = 4;
        args.dry_run = true;
        run(&args).expect("dry run");
        assert!(!path.exists(), "a dry run writes nothing");
    }

    #[test]
    fn the_baseline_summary_counts_what_it_allowed_and_what_it_no_longer_needs() {
        assert_eq!(baseline_summary(0, 0), None);
        assert_eq!(
            baseline_summary(2, 0).as_deref(),
            Some("  2 function(s) over a maximum are in the baseline")
        );
        assert_eq!(
            baseline_summary(1, 3).as_deref(),
            Some(
                "  1 function(s) over a maximum are in the baseline; 3 baseline entr(ies) no longer needed, regenerate with --write-baseline"
            )
        );
    }

    #[test]
    fn check_only_runs_the_gate_without_writing_the_report() {
        let (_dir, mut args) = setup();
        let before = fs::read_to_string(&args.file).expect("read");
        args.check_only = true;
        args.max_complexity = 4;
        let err = run(&args).unwrap_err().to_string();
        assert!(
            err.contains("pick") && err.contains("--max-complexity 4"),
            "{err}"
        );
        assert_eq!(
            fs::read_to_string(&args.file).expect("read"),
            before,
            "untouched"
        );

        args.max_complexity = 5;
        run(&args).expect("nothing is over 5");
        assert_eq!(
            fs::read_to_string(&args.file).expect("read"),
            before,
            "untouched"
        );
    }

    #[test]
    fn check_only_is_quiet_apart_from_the_failure() {
        let (_dir, mut args) = setup();
        args.check_only = true;
        args.warn_complexity = 4;
        args.max_complexity = 5;
        let (result, log) = logged(&args);
        result.expect("passes");
        assert!(
            !log.contains("risky function"),
            "the first step warns: {log}"
        );
        assert!(!log.contains("no matching function"), "{log}");
    }

    #[test]
    fn check_only_with_no_maximum_has_nothing_to_check_and_succeeds() {
        let (_dir, mut args) = setup();
        let before = fs::read_to_string(&args.file).expect("read");
        args.check_only = true;
        run(&args).expect("the default job runs the gate with every maximum off");
        assert_eq!(fs::read_to_string(&args.file).expect("read"), before);
    }

    #[test]
    fn check_only_leaves_the_matching_of_methods_to_the_annotating_step() {
        let (_dir, mut args) = setup();
        args.unmatched = Unmatched::Fail;
        args.max_complexity = 5;
        run(&args).unwrap_err();
        args.check_only = true;
        run(&args).expect("unmatched methods are not the gate's concern");
    }

    #[test]
    fn check_only_honours_a_baseline_and_can_write_one() {
        let (dir, mut args) = setup();
        let before = fs::read_to_string(&args.file).expect("read");
        args.check_only = true;
        args.max_complexity = 4;
        let path = baseline_path(&dir);

        args.write_baseline = Some(path.clone());
        run(&args).expect("recording does not fail");
        assert!(path.exists());
        assert_eq!(fs::read_to_string(&args.file).expect("read"), before);

        args.write_baseline = None;
        args.baseline = Some(path);
        run(&args).expect("baselined");
    }

    #[test]
    fn check_only_with_no_maximum_does_not_read_the_report_or_the_baseline() {
        let (dir, mut args) = setup();
        args.check_only = true;
        args.file = dir.path().join("missing.xml");
        args.baseline = Some(dir.path().join("missing.bin"));
        let (result, log) = logged(&args);
        result.expect("nothing to check, so nothing to read");
        assert!(log.contains("--baseline has no effect"), "{log}");

        args.baseline = None;
        args.write_baseline = Some(baseline_path(&dir));
        let err = run(&args).unwrap_err().to_string();
        assert!(err.contains("--max-complexity"), "still refused: {err}");
    }

    #[test]
    fn the_gate_judges_the_same_functions_as_the_annotating_step() {
        let (_dir, mut args) = setup();
        args.warn_complexity = 1;
        let annotated = execute(&args).expect("runs").functions;
        let mut gate = args;
        gate.check_only = true;
        gate.duplicates = Duplicates::Keep;
        gate.exclude_tests = true;
        gate.unmatched = Unmatched::One;
        let checked = execute(&gate).expect("runs").functions;
        assert!(!annotated.is_empty());
        assert_eq!(annotated, checked);
    }

    #[test]
    fn a_maximum_is_set_when_either_is_above_zero() {
        let (_dir, mut args) = setup();
        assert!(!args.has_maximum());
        args.max_complexity = 1;
        assert!(args.has_maximum());
        args.max_complexity = 0;
        args.max_crap = 0.5;
        assert!(args.has_maximum());
    }

    /// `parse`-like: complexity 29 over 51 lines, `covered` of them covered.
    fn parse_like(complexity: u32, covered: usize) -> FunctionMetrics {
        FunctionMetrics {
            file: "src/a.rs".into(),
            name: "parse".into(),
            line: 1,
            complexity,
            lines: 51,
            covered,
            crap: Some(metrics::crap(complexity, covered, 51)),
            is_test: false,
        }
    }

    #[test]
    fn a_baselined_function_over_only_on_complexity_is_not_failed_for_its_crap() {
        let (_dir, mut args) = setup();
        args.max_complexity = 15;
        args.max_crap = 30.0;
        let recorded = parse_like(29, 48); // CRAP 29.2, under the CRAP limit
        let baseline = Baseline::from_functions([&recorded], args.rules);

        // One line less covered: CRAP 29.4, still under the limit.
        let drifted = [parse_like(29, 47)];
        let g = gate(&args, Some(&baseline), &drifted);
        assert_eq!(g.over.len(), 1, "still over the complexity limit");
        assert!(g.failing.is_empty(), "the CRAP is not over its limit");

        // Crossing the CRAP limit is a failure, however it got there.
        let crossed = [parse_like(29, 30)];
        let g = gate(&args, Some(&baseline), &crossed);
        assert_eq!(g.failing.len(), 1, "CRAP is over its limit and worse");

        // More complex than recorded fails; the CRAP limit is off here.
        args.max_crap = 0.0;
        let more_complex = [parse_like(30, 48)];
        let g = gate(&args, Some(&baseline), &more_complex);
        assert_eq!(g.failing.len(), 1);
    }

    /// `n` baselined functions over the complexity limit (20) as they were, and
    /// as they are with one line less covered, so a CRAP of 20.0 is 20.4: worse,
    /// but under the CRAP limit and so still allowed. With `better` it is the
    /// other way round: 20.4 improved to 20.0.
    fn baselined_pairs(n: usize, better: bool) -> (Vec<FunctionMetrics>, Vec<FunctionMetrics>) {
        let covered = |covered: usize| -> Vec<FunctionMetrics> {
            (0..n)
                .map(|i| FunctionMetrics {
                    file: "src/a.rs".into(),
                    name: format!("f{i:02}"),
                    line: 1,
                    complexity: 20,
                    lines: 10,
                    covered,
                    crap: Some(metrics::crap(20, covered, 10)),
                    is_test: false,
                })
                .collect()
        };
        if better {
            (covered(9), covered(10))
        } else {
            (covered(10), covered(9))
        }
    }

    #[test]
    fn the_gate_carries_the_baselined_functions_that_moved_but_not_the_failing_ones() {
        let (_dir, mut args) = setup();
        args.max_complexity = 15;
        args.max_crap = 30.0;
        let recorded = parse_like(29, 48);
        let baseline = Baseline::from_functions([&recorded], args.rules);

        let drifted = [parse_like(29, 47)]; // CRAP 29.2 to 29.4, still allowed
        let g = gate(&args, Some(&baseline), &drifted);
        assert!(g.failing.is_empty());
        assert_eq!(g.moves.len(), 1);
        assert!(g.moves[0].degraded());

        let failing = [parse_like(31, 48)]; // more complex: fails, so not a move
        let g = gate(&args, Some(&baseline), &failing);
        assert_eq!(g.failing.len(), 1);
        assert!(g.moves.is_empty(), "a failure is reported as one");

        let g = gate(&args, None, &drifted);
        assert!(g.moves.is_empty(), "no baseline, nothing baselined");
    }

    #[test]
    fn moved_functions_are_logged_worse_as_warnings_and_better_as_notes() {
        let (_dir, mut args) = setup();
        args.max_complexity = 15;
        args.max_crap = 30.0;
        let (before, worse) = baselined_pairs(1, false);
        let baseline = Baseline::from_functions(&before, args.rules);
        let g = gate(&args, Some(&baseline), &worse);
        let (result, log) =
            crate::github::testing::logged(|| finish_with(&args, Some(&baseline), &g, &|_| {}));
        result.expect("allowed");
        assert!(
            log.contains("WARN") && log.contains("got worse") && log.contains("f00"),
            "{log}"
        );
        assert!(log.contains("20.0 -> 20.4"), "{log}");

        let (before, better) = baselined_pairs(1, true);
        let baseline = Baseline::from_functions(&before, args.rules);
        let g = gate(&args, Some(&baseline), &better);
        let (result, log) =
            crate::github::testing::logged(|| finish_with(&args, Some(&baseline), &g, &|_| {}));
        result.expect("allowed");
        assert!(
            log.contains("INFO") && log.contains("got better") && !log.contains("WARN"),
            "{log}"
        );
    }

    #[test]
    fn a_function_as_recorded_is_not_logged_and_the_log_is_capped() {
        let (_dir, mut args) = setup();
        args.max_complexity = 15;
        args.max_crap = 30.0;
        let (before, same) = baselined_pairs(1, false);
        let baseline = Baseline::from_functions(&before, args.rules);
        let g = gate(&args, Some(&baseline), &before);
        let (_, log) =
            crate::github::testing::logged(|| finish_with(&args, Some(&baseline), &g, &|_| {}));
        assert!(
            !log.contains("got worse") && !log.contains("got better"),
            "{log}"
        );
        drop(same);

        let (before, worse) = baselined_pairs(12, false);
        let baseline = Baseline::from_functions(&before, args.rules);
        let g = gate(&args, Some(&baseline), &worse);
        let (_, log) =
            crate::github::testing::logged(|| finish_with(&args, Some(&baseline), &g, &|_| {}));
        assert_eq!(log.matches("got worse").count(), MAX_RISK_WARNINGS, "{log}");
        assert!(log.contains("2 more baselined function(s) moved"), "{log}");
    }

    #[test]
    fn the_summary_file_lists_the_baselined_functions_that_moved() {
        let (dir, mut args) = setup();
        args.max_complexity = 15;
        args.max_crap = 30.0;
        let path = dir.path().join("summary.md");
        args.summary_file = Some(path.clone());
        let (before, worse) = baselined_pairs(1, false);
        let baseline = Baseline::from_functions(&before, args.rules);
        let g = gate(&args, Some(&baseline), &worse);
        finish_with(&args, Some(&baseline), &g, &|_| {}).expect("allowed");
        let md = fs::read_to_string(&path).expect("written");
        assert!(md.contains("### jci-coverage gate: passes"), "{md}");
        assert!(md.contains("#### Baselined functions that moved"), "{md}");
        assert!(md.contains("| 20 (same) | 20.0 → 20.4 (worse) |"), "{md}");
    }

    #[test]
    fn the_check_line_counts_what_is_over_and_what_the_baseline_allows() {
        assert_eq!(
            check_line(0, None),
            "complexity check: 0 function(s) over a maximum"
        );
        assert_eq!(
            check_line(3, Some(2)),
            "complexity check: 3 function(s) over a maximum, 2 allowed by the baseline"
        );
    }

    #[test]
    fn check_only_prints_one_line_with_a_baseline_too() {
        let (dir, mut args) = setup();
        args.check_only = true;
        args.max_complexity = 4;
        let path = baseline_path(&dir);
        write_baseline_file(&path, "pick", 4, Rules::Rca);
        args.baseline = Some(path);
        run(&args).expect("baselined");
    }

    #[test]
    fn a_threshold_must_be_a_finite_number_of_zero_or_more() {
        assert_eq!(threshold("30"), Ok(30.0));
        assert_eq!(threshold("0"), Ok(0.0));
        for bad in ["-5", "NaN", "inf", "abc"] {
            assert!(threshold(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_report_is_annotated_in_place_by_default() {
        let (_dir, args) = setup();
        let summary = execute(&args).expect("runs").summary;

        let xml = fs::read_to_string(&args.file).expect("read");
        assert_eq!(
            complexities(&xml),
            vec!["x::plain=1", "x::pick::&lt;u8&gt;=4", "x::generated=0"],
            "the u16 instantiation is merged into the u8 one"
        );
        // Roll-ups: class and package 5, root 5.
        assert!(xml.contains(r#"<class complexity="5" filename"#), "{xml}");
        assert!(
            xml.contains(r#"<package name="x" complexity="5">"#),
            "{xml}"
        );
        assert!(xml.contains(r#"<coverage complexity="5" "#), "{xml}");
        assert_eq!(summary.merged_away, 1);
        assert_eq!(summary.unmatched.len(), 1);
    }

    #[test]
    fn no_rollup_leaves_class_package_and_root_alone() {
        let (_dir, mut args) = setup();
        args.no_rollup = true;
        execute(&args).expect("runs");
        let xml = fs::read_to_string(&args.file).expect("read");
        assert!(xml.contains(r#"<class complexity="0" filename"#));
        assert!(xml.contains(r#"<coverage complexity="0" "#));
    }

    #[test]
    fn output_writes_elsewhere_and_leaves_the_input_alone() {
        let (dir, mut args) = setup();
        let original = fs::read_to_string(&args.file).expect("read");
        let out = dir.path().join("out/annotated.xml");
        args.output = Some(out.clone());
        execute(&args).expect("runs");
        assert_eq!(fs::read_to_string(&args.file).expect("read"), original);
        let written = fs::read_to_string(out).expect("written");
        assert_eq!(complexities(&written).len(), 3);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let (dir, mut args) = setup();
        let original = fs::read_to_string(&args.file).expect("read");
        args.dry_run = true;
        args.output = Some(dir.path().join("never.xml"));
        let summary = execute(&args).expect("runs").summary;
        assert_eq!(fs::read_to_string(&args.file).expect("read"), original);
        assert!(!dir.path().join("never.xml").exists());
        assert_eq!(summary.total, 5);
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let (dir, args) = setup();
        execute(&args).expect("runs");
        let names: Vec<_> = fs::read_dir(dir.path().join("coverage"))
            .expect("dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["cobertura.xml"]);
    }

    #[test]
    fn a_missing_report_names_the_path() {
        let (dir, mut args) = setup();
        args.file = dir.path().join("coverage/nope.xml");
        let err = execute(&args).unwrap_err();
        assert!(err.to_string().contains("nope.xml"), "{err}");
    }

    #[test]
    fn unmatched_fail_stops_before_anything_is_written() {
        let (_dir, mut args) = setup();
        args.unmatched = Unmatched::Fail;
        let original = fs::read_to_string(&args.file).expect("read");
        let err = execute(&args).unwrap_err();
        assert!(err.to_string().contains("x::generated"), "{err}");
        assert_eq!(fs::read_to_string(&args.file).expect("read"), original);
    }

    #[test]
    fn unmatched_one_counts_the_missing_method_as_one() {
        let (_dir, mut args) = setup();
        args.unmatched = Unmatched::One;
        execute(&args).expect("runs");
        let xml = fs::read_to_string(&args.file).expect("read");
        assert!(complexities(&xml).contains(&"x::generated=1".to_string()));
    }

    #[test]
    fn a_source_file_that_cannot_be_read_leaves_its_methods_unmatched() {
        let (dir, args) = setup();
        fs::remove_file(dir.path().join("src/lib.rs")).expect("remove");
        let summary = execute(&args).expect("runs").summary;
        assert_eq!(summary.matched, 0);
        assert_eq!(summary.unmatched.len(), 3);
    }

    #[test]
    fn the_source_root_locates_the_files() {
        let (dir, mut args) = setup();
        let elsewhere = dir.path().join("checkout");
        fs::create_dir_all(elsewhere.join("src")).expect("mkdir");
        fs::rename(dir.path().join("src/lib.rs"), elsewhere.join("src/lib.rs")).expect("move");
        args.source_root = elsewhere;
        assert_eq!(execute(&args).expect("runs").summary.matched, 2);
    }

    #[test]
    fn the_summary_reads_as_a_short_report() {
        let (_dir, args) = setup();
        let text = format_summary(&execute(&args).expect("runs").summary);
        assert!(text.contains("4 methods"), "{text}");
        assert!(text.contains("1 duplicate"), "{text}");
        assert!(text.contains("total 5"), "{text}");
        assert!(text.contains("max 4"), "{text}");
        assert!(text.contains("x::pick::<u8>"), "{text}");
        assert!(text.contains("1 not matched"), "{text}");
    }

    #[test]
    fn a_report_where_nothing_matched_gets_a_hint() {
        let (dir, mut args) = setup();
        args.source_root = dir.path().join("coverage");
        let summary = execute(&args).expect("runs").summary;
        assert!(nothing_matched(&summary).is_some_and(|h| h.contains("--source-root")));
        let (_dir, args) = setup();
        assert_eq!(
            nothing_matched(&execute(&args).expect("runs").summary),
            None
        );
    }

    #[test]
    fn a_report_for_another_language_fails_without_writing() {
        let (dir, args) = setup();
        let python = report("/work")
            .replace("src/lib.rs", "src/app.py")
            .replace("x::", "pkg.");
        fs::write(&args.file, &python).expect("write report");
        let err = execute(&args).unwrap_err();
        assert!(format!("{err:#}").contains("Rust"), "{err:#}");
        assert_eq!(fs::read_to_string(&args.file).expect("read"), python);
        drop(dir);
    }
}
