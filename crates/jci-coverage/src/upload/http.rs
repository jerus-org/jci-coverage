//! Multipart POST to OtterWise, with a primary/fallback endpoint pair and a
//! testable client abstraction (mirrors `report`'s `CommandRunner`). Purely
//! transport: it has no notion of what counts as success for a given
//! target — that's supplied by the caller as a predicate (see
//! `otterwise::is_success`), so a second upload target never has to touch
//! this module.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result};
use reqwest::StatusCode;

pub const DEFAULT_ENDPOINT: &str = "https://otterwise.app/ingress/upload";
const DEFAULT_FALLBACK_ENDPOINT: &str = "https://otterwise.app/ingress/upload-fallback";

/// A blocking request that hangs (accepts the connection, never responds)
/// would otherwise defeat the primary/fallback retry entirely — it only
/// triggers on an error or an unsuccessful response, never on "no response
/// at all".
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// A completed HTTP response, before any target-specific success judgement.
#[derive(Debug)]
pub struct RawResponse {
    pub status: StatusCode,
    pub body: String,
}

pub trait UploadClient {
    fn post_multipart(
        &self,
        endpoint: &str,
        fields: &[(String, String)],
        file_field: &str,
        file_path: &Path,
    ) -> Result<RawResponse>;
}

/// `--endpoint` replaces the primary endpoint outright and disables the
/// fallback — it exists for pointing at a test double, where OtterWise's
/// real fallback URL has no meaning.
pub fn resolve_endpoints(override_url: Option<&str>) -> (String, Option<String>) {
    match override_url {
        Some(url) => (url.to_string(), None),
        None => (
            DEFAULT_ENDPOINT.to_string(),
            Some(DEFAULT_FALLBACK_ENDPOINT.to_string()),
        ),
    }
}

/// Debug line for one POST attempt: where, and what came back.
fn log_attempt(endpoint: &str, result: &Result<RawResponse>) {
    match result {
        Ok(r) => tracing::debug!(
            endpoint,
            status = %r.status,
            body_bytes = r.body.len(),
            "upload attempt answered"
        ),
        Err(e) => tracing::debug!(endpoint, error = %e, "upload attempt failed"),
    }
}

/// POST to `primary`; retry once against `fallback` (if any) when the first
/// attempt errors or `is_success` rejects it. When both attempts fail, the
/// returned error/response carries diagnostics from both — not just the
/// fallback's — so a specific primary failure (e.g. an auth rejection)
/// isn't lost behind a generic fallback failure (e.g. connection refused).
pub fn post_with_fallback<C: UploadClient>(
    client: &C,
    primary: &str,
    fallback: Option<&str>,
    fields: &[(String, String)],
    file_field: &str,
    file_path: &Path,
    is_success: impl Fn(&RawResponse) -> bool,
) -> Result<RawResponse> {
    tracing::debug!(
        endpoint = primary,
        file_field,
        fields = fields.len(),
        "posting upload"
    );
    let first = client.post_multipart(primary, fields, file_field, file_path);
    log_attempt(primary, &first);
    if matches!(&first, Ok(r) if is_success(r)) {
        return first;
    }
    let Some(fallback_url) = fallback else {
        return first;
    };
    let primary_detail = match &first {
        Ok(r) => format!("{} {}", r.status, r.body),
        Err(e) => e.to_string(),
    };
    tracing::debug!(
        endpoint = fallback_url,
        "primary failed; posting to fallback"
    );
    let second = client.post_multipart(fallback_url, fields, file_field, file_path);
    log_attempt(fallback_url, &second);
    match second {
        Ok(r) if is_success(&r) => Ok(r),
        Ok(r) => Ok(RawResponse {
            status: r.status,
            body: format!(
                "primary ({primary}) failed: {primary_detail}\n\
                 fallback ({fallback_url}) failed: {} {}",
                r.status, r.body
            ),
        }),
        Err(e) => Err(e.context(format!(
            "fallback ({fallback_url}) request failed; \
             primary ({primary}) failed: {primary_detail}"
        ))),
    }
}

pub struct SystemClient {
    client: reqwest::blocking::Client,
}

impl SystemClient {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { client })
    }
}

/// The multipart form: each field as text, in order, then the report file
/// under `file_field`.
fn build_form(
    fields: &[(String, String)],
    file_field: &str,
    file_path: &Path,
) -> Result<reqwest::blocking::multipart::Form> {
    let mut form = reqwest::blocking::multipart::Form::new();
    for (key, value) in fields {
        form = form.text(key.clone(), value.clone());
    }
    // reqwest names the multipart part after the file's own name.
    tracing::debug!(
        form_field = file_field,
        filename = ?file_path.file_name(),
        path = %file_path.display(),
        "attaching report file"
    );
    form.file(file_field.to_string(), file_path)
        .with_context(|| format!("failed to attach {}", file_path.display()))
}

impl SystemClient {
    /// POST `form` to `endpoint` and read the whole response.
    fn send(
        &self,
        endpoint: &str,
        form: reqwest::blocking::multipart::Form,
    ) -> Result<RawResponse> {
        let response = self
            .client
            .post(endpoint)
            .multipart(form)
            .send()
            .with_context(|| format!("request to {endpoint} failed"))?;
        let status = response.status();
        let body = response
            .text()
            .with_context(|| format!("failed to read response body from {endpoint}"))?;
        Ok(RawResponse { status, body })
    }
}

impl UploadClient for SystemClient {
    fn post_multipart(
        &self,
        endpoint: &str,
        fields: &[(String, String)],
        file_field: &str,
        file_path: &Path,
    ) -> Result<RawResponse> {
        let form = build_form(fields, file_field, file_path)?;
        self.send(endpoint, form)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, path::PathBuf};

    use super::*;

    struct MockClient {
        calls: RefCell<Vec<String>>,
        results: RefCell<VecDeque<Result<RawResponse>>>,
    }

    impl MockClient {
        fn new(results: Vec<Result<RawResponse>>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                results: RefCell::new(results.into()),
            }
        }
    }

    impl UploadClient for MockClient {
        fn post_multipart(
            &self,
            endpoint: &str,
            _fields: &[(String, String)],
            _file_field: &str,
            _file_path: &Path,
        ) -> Result<RawResponse> {
            self.calls.borrow_mut().push(endpoint.to_string());
            self.results
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Ok(resp(200, "")))
        }
    }

    fn resp(status: u16, body: &str) -> RawResponse {
        RawResponse {
            status: StatusCode::from_u16(status).expect("valid status"),
            body: body.to_string(),
        }
    }

    /// A test-only success predicate, deliberately unrelated to OtterWise's
    /// actual marker string — proves `post_with_fallback` doesn't hardcode
    /// any target's notion of success.
    fn is_ok(r: &RawResponse) -> bool {
        r.status.is_success()
    }

    #[test]
    fn no_override_uses_the_default_primary_and_fallback() {
        let (primary, fallback) = resolve_endpoints(None);
        assert_eq!(primary, DEFAULT_ENDPOINT);
        assert_eq!(fallback.as_deref(), Some(DEFAULT_FALLBACK_ENDPOINT));
    }

    #[test]
    fn override_replaces_primary_and_drops_fallback() {
        let (primary, fallback) = resolve_endpoints(Some("http://localhost:9999"));
        assert_eq!(primary, "http://localhost:9999");
        assert_eq!(fallback, None);
    }

    #[test]
    fn success_on_primary_never_calls_fallback() {
        let client = MockClient::new(vec![Ok(resp(200, "ok"))]);
        let response = post_with_fallback(
            &client,
            "primary",
            Some("fallback"),
            &[],
            "clover",
            &PathBuf::from("f"),
            is_ok,
        )
        .expect("succeeds");
        assert!(is_ok(&response));
        assert_eq!(client.calls.borrow().as_slice(), ["primary"]);
    }

    #[test]
    fn failure_on_primary_retries_fallback_once() {
        let client = MockClient::new(vec![Ok(resp(500, "nope")), Ok(resp(200, "ok"))]);
        let response = post_with_fallback(
            &client,
            "primary",
            Some("fallback"),
            &[],
            "clover",
            &PathBuf::from("f"),
            is_ok,
        )
        .expect("succeeds");
        assert!(is_ok(&response));
        assert_eq!(client.calls.borrow().as_slice(), ["primary", "fallback"]);
    }

    #[test]
    fn network_error_on_primary_retries_fallback() {
        let client = MockClient::new(vec![
            Err(anyhow::anyhow!("connection refused")),
            Ok(resp(200, "ok")),
        ]);
        let response = post_with_fallback(
            &client,
            "primary",
            Some("fallback"),
            &[],
            "clover",
            &PathBuf::from("f"),
            is_ok,
        )
        .expect("succeeds");
        assert!(is_ok(&response));
    }

    #[test]
    fn no_fallback_configured_never_retries() {
        let client = MockClient::new(vec![Ok(resp(500, "nope"))]);
        let response = post_with_fallback(
            &client,
            "primary",
            None,
            &[],
            "clover",
            &PathBuf::from("f"),
            is_ok,
        )
        .expect("returns the primary result even on failure");
        assert!(!is_ok(&response));
        assert_eq!(client.calls.borrow().as_slice(), ["primary"]);
    }

    #[test]
    fn both_failing_preserves_the_primarys_diagnostic_not_just_the_fallbacks() {
        let client = MockClient::new(vec![
            Ok(resp(401, "invalid token")),
            Ok(resp(500, "server error")),
        ]);
        let response = post_with_fallback(
            &client,
            "primary",
            Some("fallback"),
            &[],
            "clover",
            &PathBuf::from("f"),
            is_ok,
        )
        .expect("returns a response, not an error, when both are HTTP-level responses");
        assert!(response.body.contains("invalid token"), "{}", response.body);
        assert!(response.body.contains("server error"), "{}", response.body);
    }

    #[test]
    fn both_erroring_surfaces_the_primarys_error_alongside_the_fallbacks() {
        let client = MockClient::new(vec![
            Err(anyhow::anyhow!("primary connection refused")),
            Err(anyhow::anyhow!("fallback connection refused")),
        ]);
        let err = post_with_fallback(
            &client,
            "primary",
            Some("fallback"),
            &[],
            "clover",
            &PathBuf::from("f"),
            is_ok,
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("primary connection refused"), "{msg}");
        assert!(msg.contains("fallback connection refused"), "{msg}");
    }

    /// A small report file in a temp dir.
    fn report_file(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("report.xml");
        std::fs::write(&path, content).expect("write");
        (dir, path)
    }

    fn fields() -> Vec<(String, String)> {
        vec![
            ("token".to_string(), "t0k".to_string()),
            ("branch".to_string(), "feat/x".to_string()),
        ]
    }

    #[test]
    fn the_system_client_posts_the_fields_then_the_file_as_multipart() {
        let (_dir, path) = report_file("<coverage/>");
        let (base, server) = crate::github::testing::serve(vec![("200 OK", "queued".to_string())]);
        let response = SystemClient::new()
            .expect("client")
            .post_multipart(&format!("{base}/ingress"), &fields(), "clover", &path)
            .expect("posts");
        assert_eq!(
            (response.status.as_u16(), response.body.as_str()),
            (200, "queued")
        );

        let requests = server.join().expect("server");
        let request = &requests[0];
        assert!(request.starts_with("POST /ingress "), "{request}");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: multipart/form-data; boundary="),
            "{request}"
        );
        let token = request.find("name=\"token\"").expect("token part");
        let branch = request.find("name=\"branch\"").expect("branch part");
        let file = request
            .find("name=\"clover\"; filename=\"report.xml\"")
            .expect("file part");
        assert!(
            token < branch && branch < file,
            "fields in order, file last: {request}"
        );
        assert!(
            request.contains("t0k") && request.contains("feat/x"),
            "{request}"
        );
        assert!(
            request.contains("<coverage/>"),
            "the file's content: {request}"
        );
    }

    #[test]
    fn the_system_client_returns_an_unsuccessful_status_as_a_response() {
        let (_dir, path) = report_file("x");
        let (base, server) =
            crate::github::testing::serve(vec![("500 Internal Server Error", "nope".to_string())]);
        let response = SystemClient::new()
            .expect("client")
            .post_multipart(&base, &fields(), "clover", &path)
            .expect("an HTTP error status is a response, not an error");
        assert_eq!(
            (response.status.as_u16(), response.body.as_str()),
            (500, "nope")
        );
        server.join().expect("server");
    }

    #[test]
    fn the_system_client_names_a_file_it_cannot_attach() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("absent.xml");
        let err = SystemClient::new()
            .expect("client")
            .post_multipart("http://127.0.0.1:9", &fields(), "clover", &missing)
            .unwrap_err();
        assert!(
            format!("{err:#}").contains(&format!("failed to attach {}", missing.display())),
            "{err:#}"
        );
    }

    #[test]
    fn the_system_client_names_an_endpoint_it_cannot_reach() {
        let (_dir, path) = report_file("x");
        let free = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", free.local_addr().expect("addr"));
        drop(free);
        let err = SystemClient::new()
            .expect("client")
            .post_multipart(&base, &fields(), "clover", &path)
            .unwrap_err();
        assert!(
            format!("{err:#}").contains(&format!("request to {base} failed")),
            "{err:#}"
        );
    }
}
