//! Coverage generation via `cargo-llvm-cov`.
//!
//! Not yet implemented — scaffolded in P0 so the CLI surface (flags) is
//! settled before the orchestration logic lands in P1. See
//! `crates/jci-coverage/README.md` and the project roadmap.

use anyhow::{Result, bail};
use clap::Args;

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
}

/// Run the `report` subcommand.
pub fn run(args: &ReportArgs) -> Result<()> {
    tracing::info!(?args, "report");
    bail!("jci-coverage report is not yet implemented (P1)")
}
