//! Preflight tool-presence detection.
//!
//! `report` shells out to `cargo-llvm-cov`/`cargo-nextest` as `cargo`
//! subcommand plugins rather than linking either as a library. A missing
//! plugin binary otherwise fails opaquely deep inside `cargo`'s own dispatch
//! ("no such command"); this preflight makes that fail loudly instead.

use std::process::Command;

use anyhow::{Result, bail};

/// An external tool that `jci-coverage` shells out to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// `cargo llvm-cov` — source-based coverage instrumentation (cargo-llvm-cov
    /// crate).
    CargoLlvmCov,
    /// `cargo nextest` — the nextest test runner (cargo-nextest crate).
    CargoNextest,
}

impl Tool {
    /// The binary this tool probes and invokes.
    fn binary(&self) -> &'static str {
        match self {
            Tool::CargoLlvmCov => "cargo-llvm-cov",
            Tool::CargoNextest => "cargo-nextest",
        }
    }

    /// Human-facing invocation, e.g. `cargo llvm-cov`.
    pub fn invocation(&self) -> &'static str {
        match self {
            Tool::CargoLlvmCov => "cargo llvm-cov",
            Tool::CargoNextest => "cargo nextest",
        }
    }

    /// Human-facing install guidance for one missing tool.
    fn install_hint(&self) -> String {
        format!(
            "provided by the `{}` crate — `cargo binstall {}`",
            self.binary(),
            self.binary()
        )
    }

    /// Args that make this tool's binary report its version and exit 0.
    ///
    /// Not uniformly `--version`: unlike `cargo-nextest`, `cargo-llvm-cov`
    /// requires its own subcommand name as the first argument even for
    /// `--version` — bare, it prints "expected subcommand 'llvm-cov'" and
    /// exits non-zero.
    fn probe_args(&self) -> &'static [&'static str] {
        match self {
            Tool::CargoLlvmCov => &["llvm-cov", "--version"],
            Tool::CargoNextest => &["--version"],
        }
    }

    /// Probe whether the tool's binary is present and responds successfully.
    fn is_present(&self) -> bool {
        probe(self.binary(), self.probe_args())
    }
}

/// Run `<binary> <args>`, returning true on a successful exit.
fn probe(binary: &str, args: &[&str]) -> bool {
    Command::new(binary)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Pure core: given a presence probe, return the subset of `tools` that are
/// absent, preserving input order. Separated from the subprocess probe so it
/// is unit-testable without a real toolchain installation.
pub fn missing_tools(tools: &[Tool], probe: impl Fn(&Tool) -> bool) -> Vec<Tool> {
    tools.iter().copied().filter(|t| !probe(t)).collect()
}

/// One "`<invocation>` not found on PATH (`<install hint>`)" line for a
/// missing tool. Shared by [`ensure_available`]'s hard error and
/// [`missing_tool_lines`]'s non-fatal warning, so the two stay worded
/// consistently.
fn format_missing_tool(tool: Tool) -> String {
    format!(
        "`{}` not found on PATH ({})",
        tool.invocation(),
        tool.install_hint()
    )
}

/// Ensure every tool in `tools` is available on PATH, or return an error that
/// names each missing tool and how to install it.
pub fn ensure_available(tools: &[Tool]) -> Result<()> {
    let missing = missing_tools(tools, |t| t.is_present());
    if missing.is_empty() {
        return Ok(());
    }
    let mut msg = String::from("required tool(s) not found on PATH:\n");
    for t in &missing {
        msg.push_str(&format!("  - {}\n", format_missing_tool(*t)));
    }
    bail!(msg)
}

/// Which of `tools` are missing on PATH, one formatted line each — for
/// callers (like `report --dry-run`) that want to report what's missing
/// without failing. Empty when every tool is present.
pub fn missing_tool_lines(tools: &[Tool]) -> Vec<String> {
    missing_tools(tools, |t| t.is_present())
        .into_iter()
        .map(format_missing_tool)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_llvm_cov_probes_with_its_subcommand_prefix() {
        assert_eq!(Tool::CargoLlvmCov.probe_args(), &["llvm-cov", "--version"]);
        assert_eq!(Tool::CargoNextest.probe_args(), &["--version"]);
    }

    #[test]
    fn missing_tools_reports_none_when_all_present() {
        let tools = [Tool::CargoLlvmCov, Tool::CargoNextest];
        let missing = missing_tools(&tools, |_| true);
        assert!(missing.is_empty());
    }

    #[test]
    fn missing_tools_reports_all_when_none_present() {
        let tools = [Tool::CargoLlvmCov, Tool::CargoNextest];
        let missing = missing_tools(&tools, |_| false);
        assert_eq!(missing, vec![Tool::CargoLlvmCov, Tool::CargoNextest]);
    }

    #[test]
    fn missing_tools_reports_only_absent_tool_preserving_order() {
        let tools = [Tool::CargoLlvmCov, Tool::CargoNextest];
        // Only cargo-nextest is absent.
        let missing = missing_tools(&tools, |t| *t == Tool::CargoLlvmCov);
        assert_eq!(missing, vec![Tool::CargoNextest]);
    }

    #[test]
    fn ensure_available_errors_names_missing_tool() {
        let missing = missing_tools(&[Tool::CargoNextest], |_| false);
        assert_eq!(missing, vec![Tool::CargoNextest]);
        assert_eq!(Tool::CargoNextest.invocation(), "cargo nextest");
    }

    #[test]
    fn install_hint_points_at_binstall() {
        let hint = Tool::CargoLlvmCov.install_hint();
        assert!(
            hint.contains("cargo binstall cargo-llvm-cov"),
            "got: {hint}"
        );
    }

    #[test]
    fn format_missing_tool_names_the_tool_and_the_install_hint() {
        let line = format_missing_tool(Tool::CargoNextest);
        assert!(line.contains("cargo nextest"), "got: {line}");
        assert!(line.contains("cargo binstall cargo-nextest"), "got: {line}");
        assert!(line.contains("not found on PATH"), "got: {line}");
    }
}
