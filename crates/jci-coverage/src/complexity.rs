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
    #[arg(long, value_name = "N", default_value_t = 30.0)]
    pub warn_crap: f64,

    /// Judge test code against the warning thresholds too.
    #[arg(long)]
    pub gate_tests: bool,

    /// Work out and print the summary without writing anything.
    #[arg(long)]
    pub dry_run: bool,
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
fn write_atomic(path: &Path, contents: &str) -> Result<()> {
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
        write_atomic(args.output.as_deref().unwrap_or(&args.file), &annotated)?;
    }
    let functions = metrics::function_metrics(&report, &analyses, args.rules);
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

/// Run the `complexity` subcommand.
pub fn run(args: &ComplexityArgs) -> Result<()> {
    let Outcome { summary, functions } = execute(args)?;
    if let Some(hint) = nothing_matched(&summary) {
        tracing::warn!("{hint}");
    }
    for m in summary.unmatched.iter().take(10) {
        tracing::warn!(file = %m.file, line = m.line, method = %m.name, "no matching function in the source");
    }
    let risks = metrics::risky(&functions, &args.thresholds());
    for r in &risks {
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
    print!("{}", format_summary(&summary));
    if args.warn_complexity > 0 || args.warn_crap > 0.0 {
        println!(
            "  {} function(s) at or over complexity {}, {} at or over CRAP {}",
            risks.iter().filter(|r| r.complexity).count(),
            args.warn_complexity,
            risks.iter().filter(|r| r.crap).count(),
            args.warn_crap
        );
    }
    let target = args.output.as_deref().unwrap_or(&args.file);
    if args.dry_run {
        println!("(dry run) would write {}", target.display());
    } else {
        println!("wrote {}", target.display());
    }
    Ok(())
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
