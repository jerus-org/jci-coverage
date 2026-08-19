//! Standalone coverage upload to OtterWise.
//!
//! Not yet implemented — scaffolded in P0 so the CLI surface (flags) is
//! settled before the upload logic lands in P2. See
//! `crates/jci-coverage/README.md` and the project roadmap.
//!
//! Deliberately independent of [`crate::report`]: this subcommand takes a
//! coverage file path and uploads it, with no assumption about how the file
//! was produced.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Args;

/// Parsed `upload` subcommand arguments.
#[derive(Debug, Args)]
pub struct UploadArgs {
    /// Path to the coverage report to upload.
    #[arg(long, value_name = "PATH")]
    pub file: PathBuf,

    /// Repository token.
    ///
    /// Falls back to `OTTERWISE_TOKEN` when omitted.
    #[arg(long, value_name = "TOKEN")]
    pub repo_token: Option<String>,

    /// Organisation token.
    ///
    /// Falls back to `OTTERWISE_ORG_TOKEN` when omitted. At least
    /// one of repo/org token is required.
    #[arg(long, value_name = "TOKEN")]
    pub org_token: Option<String>,

    /// Override the upload endpoint (primarily for testing).
    #[arg(long, value_name = "URL")]
    pub endpoint: Option<String>,
}

/// Run the `upload` subcommand.
pub fn run(args: &UploadArgs) -> Result<()> {
    tracing::info!(?args, "upload");
    bail!("jci-coverage upload is not yet implemented (P2)")
}
