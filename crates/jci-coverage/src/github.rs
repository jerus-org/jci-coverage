//! Posting the gate summary as a pull request comment.
//!
//! The comment is found again by the hidden marker on its first line and edited
//! in place, so a PR carries one gate comment however many times CI runs. A
//! failing gate creates the comment if there is none; a passing gate only
//! updates one that exists. Posting is an extra: every problem here is reported
//! to the caller to log, and none of it changes the gate's own result.

mod client;

use std::fmt;

use anyhow::{Result, bail};
use serde_json::{Value, json};

pub(crate) use client::ApiClient;

/// Comments per page asked of the API (its maximum).
const PER_PAGE: usize = 100;

/// Pages of comments read before giving up on finding ours.
const MAX_PAGES: usize = 50;

/// The API token. Never shown in `Debug` output or error messages.
#[derive(Clone)]
pub(crate) struct Token(String);

impl Token {
    pub(crate) fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        (!value.trim().is_empty()).then(|| Self(value.trim().to_string()))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
    Patch,
}

/// What came back: the status and the body, whatever the status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) body: String,
}

/// Sends one request to the API; the paths are relative to its base URL.
pub(crate) trait Transport {
    fn send(&self, method: Method, path: &str, body: Option<&str>) -> Result<Response>;
}

impl<T: Transport + ?Sized> Transport for &T {
    fn send(&self, method: Method, path: &str, body: Option<&str>) -> Result<Response> {
        (**self).send(method, path, body)
    }
}

/// The pull request to comment on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) repo: String,
    pub(crate) pr: u64,
}

/// Why nothing was posted, when that is not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skip {
    NoToken,
    NotAPullRequest,
    ForkedPullRequest,
    NoRepository,
}

impl Skip {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::NoToken => "no GITHUB_TOKEN is set",
            Self::NotAPullRequest => "this build is not for a pull request",
            Self::ForkedPullRequest => {
                "this pull request is from a fork, which has no access to the token"
            }
            Self::NoRepository => {
                "the repository is not known (CIRCLE_PROJECT_USERNAME and CIRCLE_PROJECT_REPONAME)"
            }
        }
    }
}

/// A repository name safe to put in a request path: `owner/name`, each part
/// made of letters, digits, `-`, `_` and `.`.
fn valid_repo(repo: &str) -> bool {
    let part = |p: &str| {
        !p.is_empty()
            && p != "."
            && p != ".."
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    repo.split_once('/')
        .is_some_and(|(owner, name)| part(owner) && part(name))
}

/// The pull request and token to post with, from the CircleCI environment.
///
/// `CIRCLE_PR_NUMBER` is only set for a pull request from a fork, whose build
/// has no secrets, so it is a reason to skip rather than a number to use.
pub(crate) fn target(
    get_env: impl Fn(&str) -> Option<String>,
) -> std::result::Result<(Target, Token), Skip> {
    let token = get_env("GITHUB_TOKEN")
        .and_then(Token::new)
        .ok_or(Skip::NoToken)?;
    if get_env("CIRCLE_PR_NUMBER").is_some_and(|n| !n.is_empty()) {
        return Err(Skip::ForkedPullRequest);
    }
    let meta = crate::upload::ci_meta::collect(&get_env);
    let pr = meta
        .pr_number
        .and_then(|n| n.parse().ok())
        .ok_or(Skip::NotAPullRequest)?;
    let repo = meta
        .repo
        .filter(|r| valid_repo(r))
        .ok_or(Skip::NoRepository)?;
    Ok((Target { repo, pr }, token))
}

/// What posting did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Posted {
    Created,
    Updated,
    /// The existing comment could not be edited, so a new one was created.
    Replaced,
    /// A passing gate with no earlier comment to update.
    Nothing,
    /// A passing gate whose earlier comment could not be edited.
    NotEditable,
}

fn comments_path(target: &Target) -> String {
    format!("/repos/{}/issues/{}/comments", target.repo, target.pr)
}

fn failed(what: &str, response: &Response) -> anyhow::Error {
    anyhow::anyhow!("{what}: GitHub answered {}", response.status)
}

/// The comment ids and bodies on one page of the pull request's comments.
fn comments_on_page(
    api: &impl Transport,
    target: &Target,
    page: usize,
) -> Result<Vec<(u64, String)>> {
    let path = format!("{}?per_page={PER_PAGE}&page={page}", comments_path(target));
    let response = api.send(Method::Get, &path, None)?;
    if response.status != 200 {
        bail!(failed(
            "could not list the pull request's comments",
            &response
        ));
    }
    let Value::Array(items) = serde_json::from_str(&response.body)? else {
        bail!("could not list the pull request's comments: the answer was not a list");
    };
    Ok(items
        .iter()
        .filter_map(|item| {
            let id = item.get("id")?.as_u64()?;
            let body = item.get("body")?.as_str()?;
            Some((id, body.to_string()))
        })
        .collect())
}

/// The newest comment that starts with the marker. The newest, because if an
/// older one cannot be edited (it is someone else's) a new one is created, and
/// that is the one to find next time.
fn find_ours(api: &impl Transport, target: &Target, marker: &str) -> Result<Option<u64>> {
    let mut found = None;
    for page in 1..=MAX_PAGES {
        let comments = comments_on_page(api, target, page)?;
        let last_page = comments.len() < PER_PAGE;
        for (id, body) in comments {
            if body.starts_with(marker) {
                found = Some(id);
            }
        }
        if last_page {
            break;
        }
    }
    Ok(found)
}

fn create(api: &impl Transport, target: &Target, markdown: &str) -> Result<()> {
    let body = json!({ "body": markdown }).to_string();
    let response = api.send(Method::Post, &comments_path(target), Some(&body))?;
    if response.status != 201 {
        bail!(failed("could not create the comment", &response));
    }
    Ok(())
}

/// Edit comment `id`; `false` when GitHub refuses (it is not ours, or is gone).
fn update(api: &impl Transport, target: &Target, id: u64, markdown: &str) -> Result<bool> {
    let path = format!("/repos/{}/issues/comments/{id}", target.repo);
    let body = json!({ "body": markdown }).to_string();
    let response = api.send(Method::Patch, &path, Some(&body))?;
    match response.status {
        200 => Ok(true),
        403 | 404 => Ok(false),
        _ => bail!(failed("could not update the comment", &response)),
    }
}

/// Post `markdown` (which starts with `marker`) to the pull request.
pub(crate) fn post(
    api: &impl Transport,
    target: &Target,
    marker: &str,
    markdown: &str,
    passing: bool,
) -> Result<Posted> {
    match (find_ours(api, target, marker)?, passing) {
        (None, true) => Ok(Posted::Nothing),
        (None, false) => create(api, target, markdown).map(|()| Posted::Created),
        (Some(id), passing) => {
            if update(api, target, id, markdown)? {
                Ok(Posted::Updated)
            } else if passing {
                Ok(Posted::NotEditable)
            } else {
                create(api, target, markdown).map(|()| Posted::Replaced)
            }
        }
    }
}

/// What was done, as a line for the log.
fn describe(posted: Posted) -> &'static str {
    match posted {
        Posted::Created => "posted the summary as a new comment on the pull request",
        Posted::Updated => "updated the summary comment on the pull request",
        Posted::Replaced => {
            "posted the summary as a new comment: the earlier one could not be edited"
        }
        Posted::Nothing => "the gate passes and there is no earlier comment to update",
        Posted::NotEditable => "the gate passes but the earlier comment could not be edited",
    }
}

/// Post the summary to the pull request this build is for, if it can be.
///
/// This never fails: a problem is logged and `None` returned, because the gate's
/// result does not depend on whether the comment was posted. `connect` makes the
/// transport once there is a pull request and a token to use.
pub(crate) fn publish<T: Transport>(
    get_env: impl Fn(&str) -> Option<String>,
    connect: impl FnOnce(Token) -> Result<T>,
    marker: &str,
    markdown: &str,
    passing: bool,
) -> Option<Posted> {
    let (target, token) = match target(&get_env) {
        Ok(found) => found,
        Err(Skip::NoToken) => {
            tracing::warn!("the summary was not posted: {}", Skip::NoToken.message());
            return None;
        }
        Err(skip) => {
            tracing::info!("the summary was not posted: {}", skip.message());
            return None;
        }
    };
    let posted = connect(token).and_then(|api| post(&api, &target, marker, markdown, passing));
    match posted {
        Ok(posted) => {
            println!("{}", describe(posted));
            Some(posted)
        }
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "could not post the summary to the pull request");
            None
        }
    }
}

#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::testing::{Mock, ok, page};
    use super::*;

    const MARKER: &str = "<!-- jci-coverage:gate -->";

    fn target() -> Target {
        Target {
            repo: "jerus-org/jci-coverage".into(),
            pr: 7,
        }
    }

    fn ours(text: &str) -> String {
        format!("{MARKER}\n{text}")
    }

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    const PR_ENV: [(&str, &str); 4] = [
        ("GITHUB_TOKEN", "ghs_secret"),
        (
            "CIRCLE_PULL_REQUEST",
            "https://github.com/jerus-org/jci-coverage/pull/7",
        ),
        ("CIRCLE_PROJECT_USERNAME", "jerus-org"),
        ("CIRCLE_PROJECT_REPONAME", "jci-coverage"),
    ];

    #[test]
    fn a_failing_gate_with_no_comment_creates_one() {
        let api = Mock::new(vec![page(&[(1, "thanks!")]), ok(201, "{}")]);
        let posted = post(&api, &target(), MARKER, &ours("fails"), false).expect("posts");
        assert_eq!(posted, Posted::Created);
        let calls = api.calls();
        assert_eq!(calls[1].0, Method::Post);
        assert_eq!(
            calls[1].1,
            "/repos/jerus-org/jci-coverage/issues/7/comments"
        );
        let sent: Value =
            serde_json::from_str(calls[1].2.as_deref().expect("a body")).expect("json");
        assert_eq!(sent["body"], ours("fails"));
    }

    #[test]
    fn a_comment_that_exists_is_edited_in_place() {
        let api = Mock::new(vec![
            page(&[(1, "thanks!"), (2, &ours("fails earlier"))]),
            ok(200, "{}"),
        ]);
        let posted = post(&api, &target(), MARKER, &ours("fails"), false).expect("posts");
        assert_eq!(posted, Posted::Updated);
        assert_eq!(api.methods(), [Method::Get, Method::Patch]);
        assert_eq!(
            api.calls()[1].1,
            "/repos/jerus-org/jci-coverage/issues/comments/2"
        );
    }

    #[test]
    fn only_a_comment_that_starts_with_the_marker_is_ours() {
        let quoted = format!("> {MARKER}\nsomeone quoting it");
        let mentioned = format!("see {MARKER}");
        let api = Mock::new(vec![page(&[(1, &quoted), (2, &mentioned)]), ok(201, "{}")]);
        let posted = post(&api, &target(), MARKER, &ours("fails"), false).expect("posts");
        assert_eq!(posted, Posted::Created);
    }

    #[test]
    fn the_newest_matching_comment_is_the_one_edited() {
        let api = Mock::new(vec![
            page(&[(3, &ours("someone else's")), (9, &ours("ours"))]),
            ok(200, "{}"),
        ]);
        post(&api, &target(), MARKER, &ours("fails"), false).expect("posts");
        assert_eq!(
            api.calls()[1].1,
            "/repos/jerus-org/jci-coverage/issues/comments/9"
        );
    }

    #[test]
    fn a_refused_edit_falls_back_to_a_new_comment_when_failing() {
        for refusal in [403, 404] {
            let api = Mock::new(vec![
                page(&[(2, &ours("old"))]),
                ok(refusal, "{}"),
                ok(201, "{}"),
            ]);
            let posted = post(&api, &target(), MARKER, &ours("fails"), false).expect("posts");
            assert_eq!(posted, Posted::Replaced, "{refusal}");
            assert_eq!(api.methods(), [Method::Get, Method::Patch, Method::Post]);
        }
    }

    #[test]
    fn a_passing_gate_only_updates_and_never_creates() {
        let none = Mock::new(vec![page(&[(1, "thanks!")])]);
        assert_eq!(
            post(&none, &target(), MARKER, &ours("passes"), true).expect("ok"),
            Posted::Nothing
        );
        assert_eq!(none.methods(), [Method::Get]);

        let there = Mock::new(vec![page(&[(2, &ours("fails"))]), ok(200, "{}")]);
        assert_eq!(
            post(&there, &target(), MARKER, &ours("passes"), true).expect("ok"),
            Posted::Updated
        );

        let refused = Mock::new(vec![page(&[(2, &ours("fails"))]), ok(403, "{}")]);
        assert_eq!(
            post(&refused, &target(), MARKER, &ours("passes"), true).expect("ok"),
            Posted::NotEditable
        );
        assert_eq!(
            refused.methods(),
            [Method::Get, Method::Patch],
            "no new comment"
        );
    }

    #[test]
    fn every_page_of_comments_is_read() {
        let full: Vec<(u64, String)> = (1..=PER_PAGE as u64)
            .map(|n| (n, "chat".to_string()))
            .collect();
        let full: Vec<(u64, &str)> = full.iter().map(|(n, b)| (*n, b.as_str())).collect();
        let second = ours("on the second page");
        let api = Mock::new(vec![page(&full), page(&[(500, &second)]), ok(200, "{}")]);
        post(&api, &target(), MARKER, &ours("fails"), false).expect("posts");
        let calls = api.calls();
        assert!(
            calls[0].1.ends_with("?per_page=100&page=1"),
            "{}",
            calls[0].1
        );
        assert!(
            calls[1].1.ends_with("?per_page=100&page=2"),
            "{}",
            calls[1].1
        );
        assert_eq!(
            calls[2].1,
            "/repos/jerus-org/jci-coverage/issues/comments/500"
        );
    }

    #[test]
    fn reading_stops_after_a_bounded_number_of_pages() {
        let full: Vec<(u64, String)> = (1..=PER_PAGE as u64)
            .map(|n| (n, "chat".to_string()))
            .collect();
        let script = (0..MAX_PAGES)
            .map(|_| {
                let full: Vec<(u64, &str)> = full.iter().map(|(n, b)| (*n, b.as_str())).collect();
                page(&full)
            })
            .chain([ok(201, "{}")])
            .collect();
        let api = Mock::new(script);
        post(&api, &target(), MARKER, &ours("fails"), false).expect("posts");
        assert_eq!(
            api.calls().len(),
            MAX_PAGES + 1,
            "the pages, then the create"
        );
    }

    #[test]
    fn api_failures_are_errors_that_name_the_status_and_not_the_token() {
        let listing = Mock::new(vec![ok(401, "Bad credentials ghs_secret")]);
        let err = format!(
            "{:#}",
            post(&listing, &target(), MARKER, "x", false).unwrap_err()
        );
        assert!(err.contains("401") && err.contains("list"), "{err}");
        assert!(!err.contains("ghs_secret"), "the body is not echoed: {err}");

        let creating = Mock::new(vec![page(&[]), ok(422, "{}")]);
        let err = post(&creating, &target(), MARKER, "x", false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("422") && err.contains("create"), "{err}");

        let updating = Mock::new(vec![page(&[(2, &ours("old"))]), ok(500, "{}")]);
        let err = post(&updating, &target(), MARKER, "x", false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("500") && err.contains("update"), "{err}");

        let garbled = Mock::new(vec![ok(200, "not json")]);
        assert!(post(&garbled, &target(), MARKER, "x", false).is_err());
        let not_a_list = Mock::new(vec![ok(200, "{}")]);
        assert!(post(&not_a_list, &target(), MARKER, "x", false).is_err());

        let transport = Mock::new(vec![Err(anyhow::anyhow!("connection refused"))]);
        assert!(post(&transport, &target(), MARKER, "x", false).is_err());
    }

    #[test]
    fn the_target_comes_from_the_circle_environment() {
        let (target, token) = target_from(&PR_ENV).expect("a pull request build");
        assert_eq!(target.repo, "jerus-org/jci-coverage");
        assert_eq!(target.pr, 7);
        assert_eq!(token.expose(), "ghs_secret");
    }

    fn target_from(vars: &[(&str, &str)]) -> std::result::Result<(Target, Token), Skip> {
        super::target(env(vars))
    }

    fn without(name: &str) -> Vec<(&'static str, &'static str)> {
        PR_ENV.iter().copied().filter(|(k, _)| *k != name).collect()
    }

    #[test]
    fn the_cases_with_nothing_to_post_to_are_skipped_with_a_reason() {
        assert_eq!(
            target_from(&without("GITHUB_TOKEN")).unwrap_err(),
            Skip::NoToken
        );
        let mut blank = without("GITHUB_TOKEN");
        blank.push(("GITHUB_TOKEN", "  "));
        assert_eq!(target_from(&blank).unwrap_err(), Skip::NoToken);
        assert_eq!(
            target_from(&without("CIRCLE_PULL_REQUEST")).unwrap_err(),
            Skip::NotAPullRequest
        );
        let mut fork = PR_ENV.to_vec();
        fork.push(("CIRCLE_PR_NUMBER", "7"));
        assert_eq!(target_from(&fork).unwrap_err(), Skip::ForkedPullRequest);
        assert_eq!(
            target_from(&without("CIRCLE_PROJECT_REPONAME")).unwrap_err(),
            Skip::NoRepository
        );
        for skip in [
            Skip::NoToken,
            Skip::NotAPullRequest,
            Skip::ForkedPullRequest,
            Skip::NoRepository,
        ] {
            assert!(!skip.message().is_empty());
        }
    }

    #[test]
    fn a_repository_name_that_could_change_the_request_path_is_refused() {
        for bad in [
            "a/b/c", "a", "/b", "a/", "a/..", "../b", "a/b?x=1", "a/b#x", "a b/c", "a/b%2f",
        ] {
            assert!(!valid_repo(bad), "{bad}");
        }
        for good in ["jerus-org/jci-coverage", "o/r.rs", "Some_Org/x-1"] {
            assert!(valid_repo(good), "{good}");
        }
        let mut evil = without("CIRCLE_PROJECT_REPONAME");
        evil.push(("CIRCLE_PROJECT_REPONAME", "x/../../user"));
        assert_eq!(target_from(&evil).unwrap_err(), Skip::NoRepository);
    }

    #[test]
    fn the_token_is_not_shown_by_debug() {
        let token = Token::new("ghs_secret").expect("a token");
        assert!(!format!("{token:?}").contains("ghs_secret"));
        assert!(Token::new("").is_none());
        let (target, token) = target_from(&PR_ENV).expect("ok");
        let shown = format!("{target:?} {token:?}");
        assert!(!shown.contains("ghs_secret"), "{shown}");
    }

    fn publish_with(vars: &[(&str, &str)], api: &Mock, passing: bool) -> (Option<Posted>, String) {
        super::testing::logged(|| {
            publish(
                env(vars),
                |token| {
                    assert_eq!(token.expose(), "ghs_secret");
                    Ok(api)
                },
                MARKER,
                &ours("fails"),
                passing,
            )
        })
    }

    #[test]
    fn publishing_posts_to_the_pull_request_this_build_is_for() {
        let api = Mock::new(vec![page(&[]), ok(201, "{}")]);
        let (posted, _) = publish_with(&PR_ENV, &api, false);
        assert_eq!(posted, Some(Posted::Created));
        assert_eq!(
            api.calls()[1].1,
            "/repos/jerus-org/jci-coverage/issues/7/comments"
        );
    }

    #[test]
    fn publishing_without_a_token_is_a_warning_and_asks_nothing_of_github() {
        let api = Mock::new(vec![]);
        let (posted, log) = publish_with(&without("GITHUB_TOKEN"), &api, false);
        assert_eq!(posted, None);
        assert!(
            log.contains("WARN") && log.contains("GITHUB_TOKEN"),
            "{log}"
        );
        assert!(api.calls().is_empty());
    }

    #[test]
    fn publishing_from_a_build_with_no_pull_request_is_noted_and_asks_nothing_of_github() {
        let api = Mock::new(vec![]);
        let (posted, log) = publish_with(&without("CIRCLE_PULL_REQUEST"), &api, false);
        assert_eq!(posted, None);
        assert!(
            log.contains("not for a pull request") && !log.contains("WARN"),
            "{log}"
        );

        let mut fork = PR_ENV.to_vec();
        fork.push(("CIRCLE_PR_NUMBER", "7"));
        let (posted, log) = publish_with(&fork, &api, false);
        assert_eq!(posted, None);
        assert!(log.contains("fork"), "{log}");
        assert!(api.calls().is_empty());
    }

    #[test]
    fn a_posting_failure_is_logged_and_never_raised() {
        let api = Mock::new(vec![ok(500, "{}")]);
        let (posted, log) = publish_with(&PR_ENV, &api, false);
        assert_eq!(posted, None);
        assert!(log.contains("WARN") && log.contains("500"), "{log}");

        let (posted, log) = super::testing::logged(|| {
            publish(
                env(&PR_ENV),
                |_| -> Result<&Mock> { Err(anyhow::anyhow!("no such host")) },
                MARKER,
                "x",
                false,
            )
        });
        assert_eq!(posted, None);
        assert!(log.contains("no such host"), "{log}");
        assert!(!log.contains("ghs_secret"), "{log}");
    }
}
