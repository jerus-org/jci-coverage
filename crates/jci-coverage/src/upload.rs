//! Standalone coverage upload to OtterWise.
//!
//! Deliberately independent of [`crate::report`]: this subcommand takes a
//! coverage file path and uploads it, with no assumption about how the file
//! was produced.

mod ci_meta;
mod diagnostics;
mod diff;
mod git_meta;
mod http;
mod otterwise;

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args;

use http::UploadClient;

/// Parsed `upload` subcommand arguments.
///
/// `Debug` is written by hand: the derived one would print `--repo-token` in
/// clear text into the CI log.
#[derive(Args)]
pub struct UploadArgs {
    /// Path to the coverage report to upload.
    #[arg(long, value_name = "PATH")]
    pub file: PathBuf,

    /// Repository token.
    ///
    /// Falls back to `OTTERWISE_TOKEN` when omitted.
    #[arg(long, value_name = "TOKEN")]
    pub repo_token: Option<String>,

    /// Override the upload endpoint (primarily for testing).
    #[arg(long, value_name = "URL")]
    pub endpoint: Option<String>,
}

impl std::fmt::Debug for UploadArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadArgs")
            .field("file", &self.file)
            .field(
                "repo_token",
                &match &self.repo_token {
                    Some(_) => "<redacted>",
                    None => "<flag not passed; OTTERWISE_TOKEN is read separately>",
                },
            )
            .field("endpoint", &self.endpoint)
            .finish()
    }
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
fn ensure_ready(file: &Path, repo_token: Option<&str>) -> Result<()> {
    if !file.is_file() {
        bail!("coverage file not found: {}", file.display());
    }
    if !token_present(repo_token) {
        bail!("no upload token: set --repo-token or OTTERWISE_TOKEN");
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
    let env_token = get_env("OTTERWISE_TOKEN");
    // `args.repo_token` is only the `--repo-token` flag, which is `None` in CI
    // where the token comes from the environment: say where the token came from
    // (never its value), so a `None` in the args line is not misread as "no
    // token".
    tracing::info!(
        repo_token = %diagnostics::describe_token(
            args.repo_token.as_deref(),
            "OTTERWISE_TOKEN",
            env_token.as_deref(),
        ),
        "upload token"
    );
    let repo_token = resolve_token(args.repo_token.as_deref(), env_token);
    ensure_ready(&args.file, repo_token.as_deref())?;

    // `None` outside a git repo (or one with no commits) — not an error, per
    // `upload`'s standalone contract (see `cli.rs`'s long_about).
    let git = git_meta::collect(cwd)?;
    let ci = ci_meta::collect(&get_env);
    let base_dir = cwd.to_string_lossy();
    // The full field list (redacted) is logged below, so this stays to a summary
    // and avoids Debug-printing the metadata structs, which would log any field
    // added to them later without a second thought.
    match git.as_ref() {
        Some(g) => tracing::debug!(
            head = %g.head.sha,
            has_parent = g.parent.is_some(),
            diff_bytes = g.diff.len(),
            "git metadata"
        ),
        None => tracing::debug!("no git metadata (not a git repository)"),
    }
    let fields = otterwise::build_fields(git.as_ref(), &ci, repo_token.as_deref(), &base_dir);
    log_request_details(&args.file, &base_dir, &fields);

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
    // The ingest is asynchronous ("Queued for processing"): what OtterWise says
    // here is the only trace of the upload we get in CI, so keep it.
    tracing::info!(status = %response.status, body = %response.body, "OtterWise response");
    println!("uploaded {} to OtterWise", args.file.display());
    Ok(())
}

/// Debug-level description of what is about to be sent: the redacted fields
/// and what the report's source paths look like against `base_dir` — the
/// first place to look when OtterWise accepts an upload but shows no coverage.
fn log_request_details(file: &Path, base_dir: &str, fields: &[(String, String)]) {
    for (key, value) in diagnostics::redact_fields(fields) {
        tracing::debug!(field = %key, value = %value, "upload field");
    }
    // Lossy, so a report with stray non-UTF-8 bytes is still summarised.
    match std::fs::read(file).map(|b| String::from_utf8_lossy(&b).into_owned()) {
        Ok(content) => {
            let summary = diagnostics::summarize_lcov(&content, base_dir);
            tracing::debug!(
                file = %file.display(),
                bytes = content.len(),
                source_files = summary.source_files,
                absolute = summary.absolute,
                under_base_dir = summary.under_base_dir,
                base_dir,
                samples = ?summary.samples,
                "coverage report"
            );
            // Relative paths are resolvable as they are; only absolute paths that
            // all miss `base_dir` suggest OtterWise cannot map the report.
            if summary.absolute > 0 && summary.under_base_dir == 0 {
                tracing::warn!(
                    base_dir,
                    samples = ?summary.samples,
                    "none of the report's absolute source paths are under base_dir; OtterWise may \
                     not be able to map them to repository files"
                );
            }
        }
        Err(e) => tracing::debug!(file = %file.display(), error = %e, "could not read report"),
    }
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
        let err = ensure_ready(Path::new("/no/such/file"), Some("t")).unwrap_err();
        assert!(err.to_string().contains("coverage file not found"));
    }

    #[test]
    fn ensure_ready_rejects_when_no_token_is_set() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("lcov.info");
        std::fs::write(&file, "").expect("write");

        let err = ensure_ready(&file, None).unwrap_err();
        assert!(err.to_string().contains("no upload token"));
    }

    #[test]
    fn ensure_ready_rejects_an_empty_string_token_the_same_as_a_missing_one() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("lcov.info");
        std::fs::write(&file, "").expect("write");

        let err = ensure_ready(&file, Some("")).unwrap_err();
        assert!(err.to_string().contains("no upload token"));
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
            endpoint: None,
        }
    }

    /// Records the multipart fields it is asked to post, then reports success.
    struct CapturingClient {
        fields: RefCell<Vec<(String, String)>>,
    }

    impl UploadClient for CapturingClient {
        fn post_multipart(
            &self,
            _endpoint: &str,
            fields: &[(String, String)],
            _file_field: &str,
            _file_path: &Path,
        ) -> Result<RawResponse> {
            *self.fields.borrow_mut() = fields.to_vec();
            Ok(resp(200, "Queued for processing"))
        }
    }

    /// `cargo llvm-cov` writes absolute source paths (`SF:/home/circleci/...`),
    /// so OtterWise needs the absolute project directory to map them back to
    /// repository files (its uploader defaults `--base-dir` to the working
    /// directory). A literal `.` strips nothing: the upload was accepted but no
    /// coverage appeared.
    #[test]
    fn upload_with_sends_the_absolute_working_directory_as_base_dir() {
        let dir = fixture_repo();
        let client = CapturingClient {
            fields: RefCell::new(Vec::new()),
        };
        let args = args_with_file(dir.path().join("lcov.info"));

        upload_with(&client, &args, dir.path(), |_| None).expect("succeeds");

        let fields = client.fields.borrow();
        let base_dir = fields
            .iter()
            .find(|(k, _)| k == "base_dir")
            .map(|(_, v)| v.as_str());
        assert_eq!(base_dir, Some(dir.path().to_str().expect("utf-8 path")));
    }

    /// `run` logs the parsed arguments. A token passed with `--repo-token` must
    /// never reach the log (CI logs are often public).
    #[test]
    fn debug_output_of_the_arguments_withholds_the_token() {
        let args = UploadArgs {
            file: PathBuf::from("coverage/lcov.info"),
            repo_token: Some("s3cret-value".to_string()),
            endpoint: None,
        };
        let shown = format!("{args:?}");
        assert!(!shown.contains("s3cret-value"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        assert!(shown.contains("coverage/lcov.info"), "{shown}");
    }

    /// The logged provenance must agree with what `resolve_token` decides, for
    /// every flag/environment combination.
    #[test]
    fn token_description_agrees_with_token_resolution() {
        let values = [None, Some(""), Some("v")];
        for flag in values {
            for env in values {
                let resolved = resolve_token(flag, env.map(String::from));
                let said = diagnostics::describe_token(flag, "OTTERWISE_TOKEN", env);
                assert_eq!(
                    resolved.is_some(),
                    !said.contains("not provided"),
                    "flag={flag:?} env={env:?}: resolved={resolved:?}, said={said}"
                );
            }
        }
    }

    /// `None` must not read as "no token": in CI the token comes from the
    /// environment, so an absent flag is the normal case.
    #[test]
    fn debug_output_explains_an_absent_token_flag() {
        let args = UploadArgs {
            file: PathBuf::from("coverage/lcov.info"),
            repo_token: None,
            endpoint: None,
        };
        let shown = format!("{args:?}");
        assert!(shown.contains("OTTERWISE_TOKEN"), "{shown}");
        assert!(!shown.contains("repo_token: None"), "{shown}");
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
