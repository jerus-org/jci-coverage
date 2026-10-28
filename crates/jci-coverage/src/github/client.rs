//! The blocking `reqwest` implementation of [`Transport`].

use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::{
    Method as HttpMethod,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue, USER_AGENT},
};

use super::{Method, Response, Token, Transport};

/// Where the API is when `GITHUB_API_URL` is not set.
pub(crate) const DEFAULT_API_URL: &str = "https://api.github.com";

/// Posting a comment is a small extra: do not hold the job up for long.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

const API_VERSION: &str = "2022-11-28";

pub(crate) struct ApiClient {
    http: reqwest::blocking::Client,
    base: String,
    token: Token,
}

/// The API base URL, if it is `https`, or `http` to this machine (for a test
/// server). The token is sent to it, so it may not be plain `http` elsewhere.
fn checked_base(url: &str) -> Result<String> {
    let url = url.trim_end_matches('/');
    let local = ["http://localhost", "http://127.0.0.1"]
        .iter()
        .any(|prefix| {
            url.strip_prefix(prefix).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with(':') || rest.starts_with('/')
            })
        });
    if url.len() > "https://".len() && url.starts_with("https://") || local {
        Ok(url.to_string())
    } else {
        bail!("the GitHub API URL must be https (or http to this machine): {url}")
    }
}

impl ApiClient {
    pub(crate) fn new(base: Option<&str>, token: Token) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("failed to build HTTP client")?;
        Self::with_client(http, base.unwrap_or(DEFAULT_API_URL), token)
    }

    pub(crate) fn with_client(
        http: reqwest::blocking::Client,
        base: &str,
        token: Token,
    ) -> Result<Self> {
        Ok(Self {
            http,
            base: checked_base(base)?,
            token,
        })
    }
}

impl Transport for ApiClient {
    fn send(&self, method: Method, path: &str, body: Option<&str>) -> Result<Response> {
        let method = match method {
            Method::Get => HttpMethod::GET,
            Method::Post => HttpMethod::POST,
            Method::Patch => HttpMethod::PATCH,
        };
        let mut auth = HeaderValue::from_str(&format!("Bearer {}", self.token.expose()))
            .context("the token is not valid in a header")?;
        auth.set_sensitive(true);
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .header(AUTHORIZATION, auth)
            .header(ACCEPT, "application/vnd.github+json")
            .header(
                USER_AGENT,
                concat!("jci-coverage/", env!("CARGO_PKG_VERSION")),
            )
            .header("X-GitHub-Api-Version", API_VERSION);
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        let response = request.send().context("the request to GitHub failed")?;
        let status = response.status().as_u16();
        let body = response.text().context("could not read GitHub's answer")?;
        Ok(Response { status, body })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use super::*;

    /// Serves one request with `answer`, and returns what it received.
    fn serve_once(
        status_line: &'static str,
        answer: &'static str,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut received = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).expect("read");
                received.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&received).to_string();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let wanted = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().expect("length"))
                        })
                        .unwrap_or(0);
                    if body.len() >= wanted {
                        break;
                    }
                }
                assert!(n > 0, "the client hung up early");
            }
            let reply = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                answer.len()
            );
            stream.write_all(reply.as_bytes()).expect("reply");
            String::from_utf8_lossy(&received).to_string()
        });
        (base, handle)
    }

    fn local_client(base: &str) -> ApiClient {
        let http = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .expect("client");
        ApiClient::with_client(http, base, Token::new("ghs_secret").expect("token"))
            .expect("client")
    }

    #[test]
    fn a_request_carries_the_headers_github_requires() {
        let (base, server) = serve_once("201 Created", "{\"id\":1}");
        let response = local_client(&base)
            .send(
                Method::Post,
                "/repos/o/r/issues/7/comments",
                Some("{\"body\":\"hi\"}"),
            )
            .expect("sends");
        assert_eq!(
            response,
            Response {
                status: 201,
                body: "{\"id\":1}".into()
            }
        );
        let request = server.join().expect("server").to_ascii_lowercase();
        assert!(
            request.starts_with("post /repos/o/r/issues/7/comments http/1.1"),
            "{request}"
        );
        for header in [
            "authorization: bearer ghs_secret",
            "accept: application/vnd.github+json",
            "x-github-api-version: 2022-11-28",
            "content-type: application/json",
        ] {
            assert!(request.contains(header), "{header}\n{request}");
        }
        assert!(
            request.contains(&format!(
                "user-agent: jci-coverage/{}",
                env!("CARGO_PKG_VERSION")
            )),
            "{request}"
        );
        assert!(request.ends_with("{\"body\":\"hi\"}"), "{request}");
    }

    #[test]
    fn a_get_has_no_body_and_a_failing_status_is_returned_not_raised() {
        let (base, server) = serve_once("404 Not Found", "{}");
        let response = local_client(&base)
            .send(
                Method::Get,
                "/repos/o/r/issues/7/comments?per_page=100&page=1",
                None,
            )
            .expect("an answer is not an error");
        assert_eq!(response.status, 404);
        let request = server.join().expect("server").to_ascii_lowercase();
        assert!(
            request.starts_with("get /repos/o/r/issues/7/comments?per_page=100&page=1 "),
            "{request}"
        );
        assert!(!request.contains("content-type"), "{request}");
    }

    #[test]
    fn a_connection_failure_is_an_error_without_the_token() {
        let free = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", free.local_addr().expect("addr"));
        drop(free);
        let err = format!(
            "{:#}",
            local_client(&base)
                .send(Method::Get, "/x", None)
                .unwrap_err()
        );
        assert!(!err.contains("ghs_secret"), "{err}");
    }

    #[test]
    fn the_token_only_goes_to_https_or_this_machine() {
        for good in [
            "https://api.github.com",
            "https://ghe.example.com/api/v3/",
            "http://localhost",
            "http://localhost:8080",
            "http://127.0.0.1:1234/x",
        ] {
            assert!(checked_base(good).is_ok(), "{good}");
        }
        for bad in [
            "http://api.github.com",
            "http://localhost.evil.com",
            "http://127.0.0.1.evil.com",
            "https://",
            "ftp://x",
            "api.github.com",
            "",
        ] {
            assert!(checked_base(bad).is_err(), "{bad}");
        }
        assert_eq!(
            checked_base("https://x.io/api/").expect("ok"),
            "https://x.io/api"
        );
    }
}
