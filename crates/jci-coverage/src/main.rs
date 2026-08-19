mod cli;
mod report;
mod upload;

use anyhow::Result;
use clap::Parser;
use cli::Cli;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Resolve the filter directive string: `rust_log` wins when it is set AND
/// non-empty; otherwise fall back to `derived` (built from -v/-q).
///
/// `EnvFilter::try_from_default_env()` alone is not enough: it succeeds
/// (with an empty, match-nothing filter) when RUST_LOG is set but EMPTY, not
/// just when it's unset — silently disabling -v/-vv with no error. That's a
/// real trap in CI, where a variable can end up exported-but-blank (a blank
/// pipeline parameter, another tool's leftover export). Treating "unset" and
/// "set-to-empty" the same way is what makes -v/-vv reliably available to
/// enrich CI output when troubleshooting is needed.
///
/// Extracted as a pure function so this contract has a pinning test: `main`
/// itself can't be unit tested (calls `Cli::parse()` and installs a
/// process-global subscriber), and env-var-based tests are flaky under a
/// parallel test runner.
fn resolve_filter_source(rust_log: Option<&str>, derived: &str) -> String {
    match rust_log {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => derived.to_string(),
    }
}

fn main() -> Result<()> {
    // Parse first: -v/-q set the level, so the subscriber cannot be built until
    // the arguments are known. RUST_LOG still wins where it is set (and non-empty).
    let cli = Cli::parse();

    let derived = format!("jci_coverage={}", cli.logging.tracing_level_filter());
    let rust_log = std::env::var("RUST_LOG").ok();
    let filter =
        tracing_subscriber::EnvFilter::new(resolve_filter_source(rust_log.as_deref(), &derived));

    // tracing_subscriber::registry().init() wires LogTracer automatically when
    // the tracing-log feature is active; calling LogTracer::init() manually
    // beforehand would panic with SetLoggerError.
    //
    // with_ansi(false): CI log viewers and `grep` want plain text, not raw
    // escape codes — colour serves a human terminal, not a troubleshooting
    // CI log.
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_ansi(false))
        .init();

    cli.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_rust_log_falls_back_to_derived() {
        assert_eq!(
            resolve_filter_source(None, "jci_coverage=info"),
            "jci_coverage=info"
        );
    }

    #[test]
    fn empty_rust_log_falls_back_to_derived_same_as_unset() {
        assert_eq!(
            resolve_filter_source(Some(""), "jci_coverage=info"),
            "jci_coverage=info"
        );
    }

    #[test]
    fn non_empty_rust_log_wins_over_derived() {
        assert_eq!(
            resolve_filter_source(Some("debug"), "jci_coverage=info"),
            "debug"
        );
    }
}
