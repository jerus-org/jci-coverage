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
    let first = client.post_multipart(primary, fields, file_field, file_path);
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
    match client.post_multipart(fallback_url, fields, file_field, file_path) {
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

impl UploadClient for SystemClient {
    fn post_multipart(
        &self,
        endpoint: &str,
        fields: &[(String, String)],
        file_field: &str,
        file_path: &Path,
    ) -> Result<RawResponse> {
        let mut form = reqwest::blocking::multipart::Form::new();
        for (key, value) in fields {
            form = form.text(key.clone(), value.clone());
        }
        form = form
            .file(file_field.to_string(), file_path)
            .with_context(|| format!("failed to attach {}", file_path.display()))?;

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
}
