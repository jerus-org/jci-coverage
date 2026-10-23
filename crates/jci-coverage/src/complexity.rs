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
mod metrics;
mod plan;

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Args;

use baseline::Baseline;
pub use counter::Rules;
use metrics::{FunctionMetrics, Thresholds};
use plan::{Analyses, Options, Plan, Summary};
pub use plan::{Duplicates, Unmatched};

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
    /// recorded. It is still warned about. Make the file with `--write-baseline`;
    /// it is a binary file for this tool, not meant to be edited.
    #[arg(long, value_name = "PATH", conflicts_with = "write_baseline")]
    pub baseline: Option<PathBuf>,

    /// Record the functions now over a maximum in this file, and do not fail.
    ///
    /// Needs `--max-complexity` or `--max-crap`. Nothing is written with
    /// `--dry-run`.
    #[arg(long, value_name = "PATH")]
    pub write_baseline: Option<PathBuf>,

    /// Judge test code against the warning and maximum thresholds too.
    #[arg(long)]
    pub gate_tests: bool,

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
    let scored = args.warn_complexity > 0
        || args.warn_crap > 0.0
        || args.max_complexity > 0
        || args.max_crap > 0.0;
    let functions = if scored {
        metrics::function_metrics(&report, &analyses, args.rules)
    } else {
        Vec::new()
    };
    Ok(Outcome { summary, functions })
}

impl ComplexityArgs {
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

/// How many functions over a maximum the failure message names.
const MAX_LISTED_FAILURES: usize = 5;

/// The failure message for the functions over a maximum, or `None` if there are
/// none. Each function is listed with the limits it is over.
fn gate_failure(over: &[metrics::Risk<'_>], args: &ComplexityArgs) -> Option<String> {
    if over.is_empty() {
        return None;
    }
    let mut out = format!("{} function(s) over a maximum:", over.len());
    for r in over.iter().take(MAX_LISTED_FAILURES) {
        let f = r.function;
        out += &format!(
            "\n  {}:{} {} (complexity {}",
            f.file, f.line, f.name, f.complexity
        );
        if let Some(crap) = f.crap {
            out += &format!(", CRAP {crap:.1}");
        }
        let mut limits = Vec::new();
        if r.complexity {
            limits.push(format!("--max-complexity {}", args.max_complexity));
        }
        if r.crap {
            limits.push(format!("--max-crap {}", args.max_crap));
        }
        out += &format!("; over {})", limits.join(", "));
    }
    if over.len() > MAX_LISTED_FAILURES {
        out += &format!("\n  ... and {} more", over.len() - MAX_LISTED_FAILURES);
    }
    Some(out)
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

/// Check the baseline options before anything is read or written, and load the
/// baseline to compare against.
fn load_baseline(args: &ComplexityArgs) -> Result<Option<Baseline>> {
    if args.write_baseline.is_some() && args.max_complexity == 0 && args.max_crap == 0.0 {
        bail!(
            "--write-baseline records the functions over a maximum: set --max-complexity or --max-crap"
        );
    }
    for path in [&args.baseline, &args.write_baseline].into_iter().flatten() {
        let report = args.output.as_deref().unwrap_or(&args.file);
        if path == &args.file || path == report {
            bail!(
                "the baseline {} is the same file as the report; give it its own path",
                path.display()
            );
        }
    }
    if args.baseline.is_some() && args.max_complexity == 0 && args.max_crap == 0.0 {
        tracing::warn!("--baseline has no effect without --max-complexity or --max-crap");
    }
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

/// Run the `complexity` subcommand.
pub fn run(args: &ComplexityArgs) -> Result<()> {
    let baseline = load_baseline(args)?;
    let Outcome { summary, functions } = execute(args)?;
    if let Some(hint) = nothing_matched(&summary) {
        tracing::warn!("{hint}");
    }
    for m in summary.unmatched.iter().take(10) {
        tracing::warn!(file = %m.file, line = m.line, method = %m.name, "no matching function in the source");
    }
    let risks = metrics::risky(&functions, &args.thresholds());
    let over = metrics::risky(&functions, &args.max_thresholds());
    // Recording a baseline accepts what is over a maximum; otherwise it is the
    // functions the baseline does not allow that fail.
    let failing: Vec<_> = if args.write_baseline.is_some() {
        Vec::new()
    } else {
        over.iter()
            .filter(|r| baseline.as_ref().is_none_or(|b| !b.allows(r.function)))
            .copied()
            .collect()
    };
    // Everything over a warn threshold or a maximum is warned about, unless it
    // is failing, so accepted and recorded functions stay visible.
    let flagged = metrics::risky(&functions, &args.thresholds().either(args.max_thresholds()));
    let warned = warnings_only(&flagged, &failing);
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
    if let Some(b) = &baseline {
        let still_over = over.iter().map(|r| r.function);
        if let Some(line) = baseline_summary(over.len() - failing.len(), b.unused(still_over)) {
            println!("{line}");
        }
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
    match gate_failure(&failing, args) {
        Some(message) => bail!("{message}"),
        None => Ok(()),
    }
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
            err.contains("pick") && err.contains("complexity 4"),
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
        assert!(err.contains("pick") && err.contains("CRAP 20.0"), "{err}");
        assert!(err.contains("over --max-crap 20"), "{err}");
        assert!(
            !err.contains("--max-complexity"),
            "only the limit it is over: {err}"
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
    fn the_failure_lists_the_worst_functions_and_counts_the_rest() {
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
        let risks = metrics::risky(&functions, &args.max_thresholds());
        let msg = gate_failure(&risks, &args).expect("fails");
        assert!(msg.starts_with("8 function(s) over a maximum"), "{msg}");
        assert!(
            msg.contains("f8") && !msg.contains("f1 "),
            "worst first, capped: {msg}"
        );
        assert!(msg.contains("and 3 more"), "{msg}");
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
