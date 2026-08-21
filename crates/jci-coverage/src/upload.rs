//! Standalone coverage upload to OtterWise.
//!
//! Deliberately independent of [`crate::report`]: this subcommand takes a
//! coverage file path and uploads it, with no assumption about how the file
//! was produced.

mod ci_meta;
mod diff;
mod git_meta;
mod http;
mod otterwise;

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args;

use http::UploadClient;

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

/// CLI flag wins; falls back to the given env value. An empty string from
/// either source counts as absent (matches `report`'s set-but-empty-means-
/// unset convention), so a CI template that expands an unset secret to `""`
/// still falls through to a real value instead of silently winning.
fn resolve_token(cli_flag: Option<&str>, env: Option<String>) -> Option<String> {
    match cli_flag {
        Some(v) if !v.is_empty() => Some(v.to_string()),
        _ => env.filter(|v| !v.is_empty()),
    }
}

fn token_present(token: Option<&str>) -> bool {
    token.is_some_and(|v| !v.is_empty())
}

/// Fail before touching the network: an absent coverage file or missing
/// token both indicate misconfiguration, not something an upload retry
/// would fix. Checks emptiness directly (not just `is_none`) so this stays
/// a real gate even if a caller skips `resolve_token`.
fn ensure_ready(file: &Path, repo_token: Option<&str>, org_token: Option<&str>) -> Result<()> {
    if !file.is_file() {
        bail!("coverage file not found: {}", file.display());
    }
    if !token_present(repo_token) && !token_present(org_token) {
        bail!(
            "no upload token: set --repo-token/--org-token or \
             OTTERWISE_TOKEN/OTTERWISE_ORG_TOKEN"
        );
    }
    Ok(())
}

/// Field name OtterWise's ingress endpoint expects the coverage file under,
/// regardless of its format.
const FILE_FIELD: &str = "clover";

/// Core orchestration, testable without real tools or network — the caller
/// supplies the client and env lookup (mirrors `report_with`'s split).
fn upload_with<C: UploadClient>(
    client: &C,
    args: &UploadArgs,
    cwd: &Path,
    get_env: impl Fn(&str) -> Option<String>,
) -> Result<()> {
    let repo_token = resolve_token(args.repo_token.as_deref(), get_env("OTTERWISE_TOKEN"));
    let org_token = resolve_token(args.org_token.as_deref(), get_env("OTTERWISE_ORG_TOKEN"));
    ensure_ready(&args.file, repo_token.as_deref(), org_token.as_deref())?;

    // `None` outside a git repo (or one with no commits) — not an error, per
    // `upload`'s standalone contract (see `cli.rs`'s long_about).
    let git = git_meta::collect(cwd)?;
    let ci = ci_meta::collect(&get_env);
    let fields = otterwise::build_fields(
        git.as_ref(),
        &ci,
        repo_token.as_deref(),
        org_token.as_deref(),
    );

    let (primary, fallback) = http::resolve_endpoints(args.endpoint.as_deref());
    let response = http::post_with_fallback(
        client,
        &primary,
        fallback.as_deref(),
        &fields,
        FILE_FIELD,
        &args.file,
        otterwise::is_success,
    )?;

    if !otterwise::is_success(&response) {
        bail!(
            "OtterWise upload failed: {} {}",
            response.status,
            response.body
        );
    }
    println!("uploaded {} to OtterWise", args.file.display());
    Ok(())
}

/// Run the `upload` subcommand.
pub fn run(args: &UploadArgs) -> Result<()> {
    tracing::info!(?args, "upload");
    let cwd = std::env::current_dir()?;
    let client = http::SystemClient::new()?;
    upload_with(&client, args, &cwd, |k| std::env::var(k).ok())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::Path};

    use tempfile::tempdir;

    use super::*;
    use http::RawResponse;

    #[test]
    fn resolve_token_prefers_the_cli_flag_over_env() {
        assert_eq!(
            resolve_token(Some("cli"), Some("env".to_string())),
            Some("cli".to_string())
        );
    }

    #[test]
    fn resolve_token_falls_back_to_env_when_flag_is_absent() {
        assert_eq!(
            resolve_token(None, Some("env".to_string())),
            Some("env".to_string())
        );
    }

    #[test]
    fn resolve_token_is_none_when_neither_is_set() {
        assert_eq!(resolve_token(None, None), None);
    }

    #[test]
    fn resolve_token_falls_back_to_env_when_the_cli_flag_is_an_empty_string() {
        assert_eq!(
            resolve_token(Some(""), Some("env".to_string())),
            Some("env".to_string())
        );
    }

    #[test]
    fn resolve_token_is_none_when_both_are_empty_strings() {
        assert_eq!(resolve_token(Some(""), Some(String::new())), None);
    }

    #[test]
    fn ensure_ready_rejects_a_missing_coverage_file() {
        let err = ensure_ready(Path::new("/no/such/file"), Some("t"), None).unwrap_err();
        assert!(err.to_string().contains("coverage file not found"));
    }

    #[test]
    fn ensure_ready_rejects_when_no_token_is_set() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("lcov.info");
        std::fs::write(&file, "").expect("write");

        let err = ensure_ready(&file, None, None).unwrap_err();
        assert!(err.to_string().contains("no upload token"));
    }

    #[test]
    fn ensure_ready_rejects_an_empty_string_token_the_same_as_a_missing_one() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("lcov.info");
        std::fs::write(&file, "").expect("write");

        let err = ensure_ready(&file, Some(""), Some("")).unwrap_err();
        assert!(err.to_string().contains("no upload token"));
    }

    #[test]
    fn ensure_ready_accepts_org_token_without_a_repo_token() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("lcov.info");
        std::fs::write(&file, "").expect("write");

        ensure_ready(&file, None, Some("org")).expect("org token alone is enough");
    }

    struct MockClient {
        outcome: RefCell<Option<Result<RawResponse>>>,
    }

    fn resp(status: u16, body: &str) -> RawResponse {
        RawResponse {
            status: reqwest::StatusCode::from_u16(status).expect("valid status"),
            body: body.to_string(),
        }
    }

    impl UploadClient for MockClient {
        fn post_multipart(
            &self,
            _endpoint: &str,
            _fields: &[(String, String)],
            _file_field: &str,
            _file_path: &Path,
        ) -> Result<RawResponse> {
            self.outcome
                .borrow_mut()
                .take()
                .unwrap_or_else(|| Ok(resp(200, "Queued for processing")))
        }
    }

    /// A one-commit repo with a coverage file already written, ready to feed
    /// `upload_with` end to end.
    fn fixture_repo() -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("init");
        std::fs::write(dir.path().join("a.txt"), "one\n").expect("write");
        let sig = git2::Signature::now("Test", "test@example.com").expect("sig");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("a.txt")).expect("add");
        let tree_id = index.write_tree().expect("write_tree");
        let tree = repo.find_tree(tree_id).expect("find_tree");
        repo.commit(Some("HEAD"), &sig, &sig, "first commit", &tree, &[])
            .expect("commit");
        std::fs::write(dir.path().join("lcov.info"), "TN:\n").expect("write coverage file");
        dir
    }

    fn args_with_file(file: PathBuf) -> UploadArgs {
        UploadArgs {
            file,
            repo_token: Some("rt".to_string()),
            org_token: None,
            endpoint: None,
        }
    }

    #[test]
    fn upload_with_reports_success_when_the_client_reports_success() {
        let dir = fixture_repo();
        let client = MockClient {
            outcome: RefCell::new(Some(Ok(resp(200, "Queued for processing")))),
        };
        let args = args_with_file(dir.path().join("lcov.info"));

        upload_with(&client, &args, dir.path(), |_| None).expect("succeeds");
    }

    #[test]
    fn upload_with_fails_when_the_client_reports_failure() {
        let dir = fixture_repo();
        let client = MockClient {
            outcome: RefCell::new(Some(Ok(resp(200, "bad token")))),
        };
        let mut args = args_with_file(dir.path().join("lcov.info"));
        // No fallback configured, so the canned failure is the final result
        // rather than being retried against a second (defaulting-to-success)
        // mock call.
        args.endpoint = Some("http://localhost:9999".to_string());

        let err = upload_with(&client, &args, dir.path(), |_| None).unwrap_err();
        assert!(err.to_string().contains("bad token"));
    }

    #[test]
    fn upload_with_never_calls_the_client_when_preflight_fails() {
        let dir = fixture_repo();
        let client = MockClient {
            outcome: RefCell::new(None),
        };
        let mut args = args_with_file(dir.path().join("lcov.info"));
        args.repo_token = None;

        let err = upload_with(&client, &args, dir.path(), |_| None).unwrap_err();
        assert!(err.to_string().contains("no upload token"));
    }

    #[test]
    fn upload_with_never_calls_the_client_when_the_repo_token_is_an_empty_string() {
        let dir = fixture_repo();
        let client = MockClient {
            outcome: RefCell::new(None),
        };
        let mut args = args_with_file(dir.path().join("lcov.info"));
        args.repo_token = Some(String::new());

        let err = upload_with(&client, &args, dir.path(), |_| None).unwrap_err();
        assert!(err.to_string().contains("no upload token"));
    }

    #[test]
    fn upload_with_succeeds_outside_any_git_repo() {
        // Proves `upload` is genuinely standalone: no `.git` anywhere under a
        // fresh tempdir, yet the upload still goes through.
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("lcov.info");
        std::fs::write(&file, "TN:\n").expect("write coverage file");
        let client = MockClient {
            outcome: RefCell::new(Some(Ok(resp(200, "Queued for processing")))),
        };
        let args = args_with_file(file);

        upload_with(&client, &args, dir.path(), |_| None)
            .expect("a missing git repo must not fail the upload");
    }
}
