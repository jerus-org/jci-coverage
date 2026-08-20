//! Coverage generation via `cargo-llvm-cov`.
//!
//! Orchestrates `cargo llvm-cov` (and, when the nextest runner is selected,
//! `cargo llvm-cov nextest`) as a subprocess — functionally equivalent to
//! `circleci-toolkit`'s `code_coverage` job. Writes `coverage/lcov.info`,
//! then prints a terminal summary via a second `cargo llvm-cov report
//! --summary-only` invocation (which reads the profile data the first
//! invocation collected, rather than re-running anything).

use std::{
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::preflight::{self, Tool};

/// Where the lcov report is written, relative to the working directory.
pub const OUTPUT_PATH: &str = "coverage/lcov.info";

/// Test runner `report` drives through `cargo-llvm-cov`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Runner {
    /// `cargo llvm-cov` — plain `cargo test`.
    Test,
    /// `cargo llvm-cov nextest` — needs `cargo-nextest` too.
    Nextest,
}

/// Parsed `report` subcommand arguments.
#[derive(Debug, Args)]
pub struct ReportArgs {
    /// Limit coverage to one workspace package.
    #[arg(long, value_name = "NAME")]
    pub package: Option<String>,

    /// Test runner to drive coverage with.
    #[arg(long, value_enum, default_value_t = Runner::Test)]
    pub runner: Runner,

    /// Nextest profile to use.
    ///
    /// Only meaningful with `--runner nextest`.
    #[arg(long, value_name = "NAME")]
    pub nextest_profile: Option<String>,

    /// Disable `--all-features` (on by default).
    ///
    /// Matches `toolkit/code_coverage`, so instrumented subprocess
    /// coverage is captured.
    #[arg(long)]
    pub no_all_features: bool,

    /// Preview the cargo-llvm-cov invocation without running it.
    ///
    /// Never fails on a missing tool or touches the filesystem — works
    /// with zero tool setup, so it's usable for previewing the exact
    /// argv, and for anything (CI smoke checks, this crate's own CLI
    /// tests) that wants to exercise this command's logging without
    /// cargo-llvm-cov installed at all. Still warns (without failing)
    /// when a required tool is missing, so the preview stays honest
    /// about whether a real run would actually work.
    #[arg(long)]
    pub dry_run: bool,
}

/// Build the `cargo llvm-cov ...` argv that generates the lcov report.
fn build_report_args(args: &ReportArgs) -> Vec<String> {
    let mut argv = vec!["llvm-cov".to_string()];

    if args.runner == Runner::Nextest {
        argv.push("nextest".to_string());
        // Only meaningful with the nextest runner — silently dropped for
        // `--runner test`, where a plain `cargo test` run has no concept of
        // nextest profiles.
        if let Some(profile) = &args.nextest_profile {
            argv.push("--profile".to_string());
            argv.push(profile.clone());
        }
    }

    if let Some(package) = &args.package {
        argv.push("--package".to_string());
        argv.push(package.clone());
    }

    if !args.no_all_features {
        argv.push("--all-features".to_string());
    }

    argv.push("--lcov".to_string());
    argv.push("--output-path".to_string());
    argv.push(OUTPUT_PATH.to_string());

    argv
}

/// Build the `cargo llvm-cov report --summary-only` argv.
fn build_summary_args() -> Vec<String> {
    vec![
        "llvm-cov".to_string(),
        "report".to_string(),
        "--summary-only".to_string(),
    ]
}

/// The tools `report` needs, given the selected runner.
fn preflight_tools(args: &ReportArgs) -> Vec<Tool> {
    let mut tools = vec![Tool::CargoLlvmCov];
    if args.runner == Runner::Nextest {
        tools.push(Tool::CargoNextest);
    }
    tools
}

/// Abstraction over running `cargo <argv>`, so orchestration is testable
/// without a real cargo-llvm-cov invocation.
pub trait CommandRunner {
    fn run(&self, argv: &[String], cwd: &Path) -> Result<bool>;
}

/// Set (to any value) on every real `cargo llvm-cov` child this crate spawns,
/// and inherited by everything *that* process in turn spawns — including,
/// transitively, `cargo llvm-cov`'s own internal `cargo test`, which runs
/// this crate's `cli_tests` trycmd suite. If any fixture in that suite were
/// to invoke `jci-coverage report` for real (not `--dry-run`), it would see
/// this var already set and refuse (see [`is_unsafe_recursive_invocation`])
/// instead of spawning `cargo llvm-cov` again from inside an already-running
/// `cargo llvm-cov` — a recursion that doesn't terminate on its own and
/// exhausts memory rather than erroring cleanly. Confirmed the hard way.
const RECURSION_GUARD_ENV: &str = "JCI_COVERAGE_REPORT_ACTIVE";

/// Runs commands as real subprocesses, inheriting stdio rather than
/// buffering it: `cargo llvm-cov` can run for minutes (compiling with
/// instrumentation, running the full test suite), and live progress in the
/// CI log is far more useful than output withheld until the process exits.
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, argv: &[String], cwd: &Path) -> Result<bool> {
        let status = Command::new("cargo")
            .args(argv)
            .current_dir(cwd)
            .env(RECURSION_GUARD_ENV, "1")
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .with_context(|| format!("failed to run `cargo {}`", argv.join(" ")))?;
        Ok(status.success())
    }
}

/// Whether `report` must refuse to run for real because it's already nested
/// inside another real `report` invocation. Dry runs never spawn a real
/// subprocess, so nesting one inside another (or inside a real run) is
/// always safe — the guard only applies when `dry_run` is false. Extracted
/// as a pure function so this decision has a test independent of real
/// environment variables and subprocesses; `run` (untestable — reads real
/// env, spawns real processes) just wires it up.
fn is_unsafe_recursive_invocation(dry_run: bool, guard_env_is_set: bool) -> bool {
    !dry_run && guard_env_is_set
}

/// Whether the recursion-guard env var counts as active — set AND
/// non-empty, matching this crate's established set-but-empty-means-unset
/// convention (see `resolve_filter_source` in `main.rs`). An exported-but-
/// blank var (a templated CI value left empty, an incomplete `unset` in some
/// shells) must not false-positive block a legitimate top-level run.
fn guard_is_active(raw: Option<&str>) -> bool {
    matches!(raw, Some(v) if !v.is_empty())
}

/// Prints the argv it would execute and returns success without touching a
/// real subprocess — backs `--dry-run`.
pub struct DryRunRunner;

impl CommandRunner for DryRunRunner {
    fn run(&self, argv: &[String], _cwd: &Path) -> Result<bool> {
        println!("(dry run) would run: cargo {}", argv.join(" "));
        Ok(true)
    }
}

/// Run the `report` subcommand.
pub fn run(args: &ReportArgs) -> Result<()> {
    let guard_env = std::env::var(RECURSION_GUARD_ENV).ok();
    if is_unsafe_recursive_invocation(args.dry_run, guard_is_active(guard_env.as_deref())) {
        bail!(
            "refusing to run: {RECURSION_GUARD_ENV} is already set, meaning \
             this is nested inside another real `report` run (most likely \
             cargo-llvm-cov re-running this crate's own test suite, which \
             then tried to invoke `report` again) — running for real here \
             would recurse without bound. Use --dry-run if this is \
             intentional."
        );
    }
    let cwd = std::env::current_dir()?;
    if args.dry_run {
        // Never hard-fails on a missing tool (that's what keeps a dry run
        // usable with zero tool setup — see the safety comment on
        // is_unsafe_recursive_invocation's caller, and
        // tests/cmd/logging_empty_rust_log_still_logs.trycmd, which relies
        // on exactly that). But a preview that silently hid "this would
        // actually fail" wouldn't be an effective preview, so still report
        // what's missing — just as a warning, not an error.
        for line in preflight::missing_tool_lines(&preflight_tools(args)) {
            println!("(dry run) warning: {line}");
        }
        report_with(&DryRunRunner, &cwd, args)
    } else {
        preflight::ensure_available(&preflight_tools(args))?;
        report_with(&SystemRunner, &cwd, args)
    }
}

/// Core orchestration, testable without real tools on PATH — preflighting is
/// the caller's job (see `run`), mirroring jci-audit's `check_with`/
/// `run_check` split.
fn report_with<R: CommandRunner>(runner: &R, cwd: &Path, args: &ReportArgs) -> Result<()> {
    if !args.dry_run {
        std::fs::create_dir_all(cwd.join("coverage"))
            .context("failed to create coverage/ directory")?;
    }

    tracing::info!(?args, "report");

    if !runner.run(&build_report_args(args), cwd)? {
        bail!("cargo llvm-cov failed to generate {OUTPUT_PATH}");
    }
    if args.dry_run {
        println!("(dry run) would write {}", cwd.join(OUTPUT_PATH).display());
    } else {
        println!("wrote {}", cwd.join(OUTPUT_PATH).display());
    }

    if !runner.run(&build_summary_args(), cwd)? {
        bail!("cargo llvm-cov report --summary-only failed");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque};

    use tempfile::tempdir;

    use super::*;

    fn args(
        package: Option<&str>,
        runner: Runner,
        nextest_profile: Option<&str>,
        no_all_features: bool,
    ) -> ReportArgs {
        ReportArgs {
            package: package.map(String::from),
            runner,
            nextest_profile: nextest_profile.map(String::from),
            no_all_features,
            dry_run: false,
        }
    }

    #[test]
    fn report_args_defaults() {
        let a = args(None, Runner::Test, None, false);
        assert_eq!(
            build_report_args(&a),
            vec![
                "llvm-cov",
                "--all-features",
                "--lcov",
                "--output-path",
                OUTPUT_PATH
            ]
        );
    }

    #[test]
    fn report_args_nextest_runner_inserts_nextest() {
        let a = args(None, Runner::Nextest, None, false);
        assert_eq!(
            build_report_args(&a),
            vec![
                "llvm-cov",
                "nextest",
                "--all-features",
                "--lcov",
                "--output-path",
                OUTPUT_PATH
            ]
        );
    }

    #[test]
    fn report_args_nextest_profile_only_applies_with_nextest_runner() {
        let a = args(None, Runner::Nextest, Some("ci"), false);
        assert_eq!(
            build_report_args(&a),
            vec![
                "llvm-cov",
                "nextest",
                "--profile",
                "ci",
                "--all-features",
                "--lcov",
                "--output-path",
                OUTPUT_PATH
            ]
        );

        // The flag is documented as only meaningful with --runner nextest;
        // pin that it's silently dropped, not passed to a plain `cargo test`
        // run where it would be meaningless (or worse, misapplied).
        let ignored = args(None, Runner::Test, Some("ci"), false);
        assert_eq!(
            build_report_args(&ignored),
            vec![
                "llvm-cov",
                "--all-features",
                "--lcov",
                "--output-path",
                OUTPUT_PATH
            ]
        );
    }

    #[test]
    fn report_args_package_filter() {
        let a = args(Some("jci-coverage"), Runner::Test, None, false);
        assert_eq!(
            build_report_args(&a),
            vec![
                "llvm-cov",
                "--package",
                "jci-coverage",
                "--all-features",
                "--lcov",
                "--output-path",
                OUTPUT_PATH
            ]
        );
    }

    #[test]
    fn report_args_no_all_features_omits_the_flag() {
        let a = args(None, Runner::Test, None, true);
        assert_eq!(
            build_report_args(&a),
            vec!["llvm-cov", "--lcov", "--output-path", OUTPUT_PATH]
        );
    }

    #[test]
    fn summary_args_are_fixed() {
        assert_eq!(
            build_summary_args(),
            vec!["llvm-cov", "report", "--summary-only"]
        );
    }

    #[test]
    fn preflight_tools_test_runner_needs_only_llvm_cov() {
        let a = args(None, Runner::Test, None, false);
        assert_eq!(preflight_tools(&a), vec![Tool::CargoLlvmCov]);
    }

    #[test]
    fn preflight_tools_nextest_runner_needs_both() {
        let a = args(None, Runner::Nextest, None, false);
        assert_eq!(
            preflight_tools(&a),
            vec![Tool::CargoLlvmCov, Tool::CargoNextest]
        );
    }

    /// Records every invocation and returns queued results in order, so
    /// orchestration (call count, argv, sequencing) is testable without a
    /// real cargo-llvm-cov on PATH.
    struct MockRunner {
        calls: RefCell<Vec<Vec<String>>>,
        results: RefCell<VecDeque<bool>>,
    }

    impl MockRunner {
        fn new(results: Vec<bool>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                results: RefCell::new(results.into()),
            }
        }
    }

    impl CommandRunner for MockRunner {
        fn run(&self, argv: &[String], _cwd: &Path) -> Result<bool> {
            self.calls.borrow_mut().push(argv.to_vec());
            Ok(self.results.borrow_mut().pop_front().unwrap_or(true))
        }
    }

    #[test]
    fn report_with_creates_the_coverage_directory() {
        let dir = tempdir().expect("tempdir");
        let runner = MockRunner::new(vec![true, true]);
        let a = args(None, Runner::Test, None, false);

        report_with(&runner, dir.path(), &a).expect("succeeds");

        assert!(dir.path().join("coverage").is_dir());
    }

    #[test]
    fn report_with_runs_report_then_summary_in_order() {
        let dir = tempdir().expect("tempdir");
        let runner = MockRunner::new(vec![true, true]);
        let a = args(None, Runner::Test, None, false);

        report_with(&runner, dir.path(), &a).expect("succeeds");

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], build_report_args(&a));
        assert_eq!(calls[1], build_summary_args());
    }

    #[test]
    fn report_with_short_circuits_when_report_command_fails() {
        let dir = tempdir().expect("tempdir");
        let runner = MockRunner::new(vec![false]);
        let a = args(None, Runner::Test, None, false);

        let err = report_with(&runner, dir.path(), &a).unwrap_err();

        assert!(err.to_string().contains("cargo llvm-cov failed"));
        // The summary step depends on the first step's profile data — never
        // run it after a failed report generation.
        assert_eq!(runner.calls.borrow().len(), 1);
    }

    #[test]
    fn report_with_fails_when_summary_command_fails() {
        let dir = tempdir().expect("tempdir");
        let runner = MockRunner::new(vec![true, false]);
        let a = args(None, Runner::Test, None, false);

        let err = report_with(&runner, dir.path(), &a).unwrap_err();

        assert!(err.to_string().contains("summary-only failed"));
    }

    #[test]
    fn recursion_guard_blocks_a_real_run_nested_inside_another() {
        assert!(is_unsafe_recursive_invocation(false, true));
    }

    #[test]
    fn guard_env_unset_is_not_active() {
        assert!(!guard_is_active(None));
    }

    #[test]
    fn guard_env_set_but_empty_is_not_active() {
        // Matches resolve_filter_source's convention in main.rs: an
        // exported-but-blank var (a templated CI value left empty, an
        // incomplete `unset`) must not false-positive block a legitimate
        // top-level run.
        assert!(!guard_is_active(Some("")));
    }

    #[test]
    fn guard_env_set_non_empty_is_active() {
        assert!(guard_is_active(Some("1")));
    }

    #[test]
    fn recursion_guard_allows_a_normal_first_run() {
        assert!(!is_unsafe_recursive_invocation(false, false));
    }

    #[test]
    fn recursion_guard_never_blocks_dry_run() {
        // Dry runs never spawn a real subprocess, so nesting one inside
        // another is always safe regardless of the guard env var.
        assert!(!is_unsafe_recursive_invocation(true, true));
        assert!(!is_unsafe_recursive_invocation(true, false));
    }

    #[test]
    fn dry_run_runner_succeeds_without_a_real_subprocess() {
        // No real cargo-llvm-cov/cargo-nextest on PATH is required for this
        // to pass — that's the point: DryRunRunner never shells out.
        let dir = tempdir().expect("tempdir");
        let result = DryRunRunner.run(
            &["llvm-cov".to_string(), "--version".to_string()],
            dir.path(),
        );
        assert!(matches!(result, Ok(true)));
    }

    #[test]
    fn report_with_dry_run_runs_report_then_summary_via_dry_run_runner() {
        let dir = tempdir().expect("tempdir");
        let mut a = args(None, Runner::Test, None, false);
        a.dry_run = true;

        // DryRunRunner itself has no call-recording; reuse report_with's
        // existing sequencing contract (already pinned against MockRunner in
        // report_with_runs_report_then_summary_in_order) — here we only need
        // to confirm dry-run mode succeeds end-to-end without a real tool.
        report_with(&DryRunRunner, dir.path(), &a).expect("succeeds without any real subprocess");
    }

    #[test]
    fn report_with_dry_run_does_not_touch_the_filesystem() {
        // A dry run that still creates coverage/ isn't a dry run — and it
        // would leave stray directories behind wherever a CLI-level test
        // happens to invoke it from.
        let dir = tempdir().expect("tempdir");
        let mut a = args(None, Runner::Test, None, false);
        a.dry_run = true;

        report_with(&DryRunRunner, dir.path(), &a).expect("succeeds");

        assert!(
            !dir.path().join("coverage").exists(),
            "dry run must not create coverage/"
        );
    }
}
