//! Maps git/CI metadata onto the multipart text fields OtterWise's
//! `/ingress/upload` endpoint expects. Isolated from the CLI and HTTP layers
//! so a future upload target is a new module, not a rewrite of `upload`'s
//! surface.

use super::{ci_meta::CiMetadata, git_meta::GitMetadata, http::RawResponse};

/// Substring OtterWise's response body contains on a successful ingest.
const SUCCESS_MARKER: &str = "Queued for processing";

/// OtterWise-specific success determination: a 2xx status alone isn't
/// enough (a WAF/CDN in front of it can 2xx an error page), and a body
/// match alone isn't enough (a 401/5xx body could echo the marker back).
/// Kept here, not in `http`, so a second upload target defines its own
/// notion of success instead of touching the transport layer.
pub fn is_success(response: &RawResponse) -> bool {
    response.status.is_success() && response.body.contains(SUCCESS_MARKER)
}

/// `git` is `None` when `upload` is run outside a git repo (or one with no
/// commits yet) — `upload`'s own docs (see `cli.rs`) call it standalone, so
/// that's not a failure: every git-derived field below is simply omitted.
///
/// `git_base_branch` (the PR's merge target) has no CircleCI env-var
/// equivalent, unlike every other field here — CircleCI doesn't expose it.
/// Left unsent; OtterWise falls back to full-file coverage instead of diff
/// coverage until this is filled in. See ROADMAP.md.
pub fn build_fields(
    git: Option<&GitMetadata>,
    ci: &CiMetadata,
    repo_token: Option<&str>,
    org_token: Option<&str>,
) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut push = |k: &str, v: Option<&str>| {
        if let Some(v) = v
            && !v.is_empty()
        {
            fields.push((k.to_string(), v.to_string()));
        }
    };

    push("ci_provider", Some(ci.provider));
    push("ci_job", ci.job.as_deref());
    push("ci_build", ci.build_num.as_deref());
    push("ci_author", ci.username.as_deref());
    push("repo_token", repo_token);
    push("org_token", org_token);
    push("git_pr", ci.pr_number.as_deref());
    push("git_branch", ci.branch.as_deref());
    push("base_dir", Some("."));

    if let Some(git) = git {
        push("diff", Some(&git.diff));
        push("git_repo", git.remote_url.as_deref());
        push("git_head_commit", Some(&git.head.sha));
        push("git_head_branch", git.branch.as_deref());
        push("head_commit_author_name", Some(&git.head.author_name));
        push("head_commit_author_email", Some(&git.head.author_email));
        push("head_commit_author_message", Some(&git.head.message));
        push("head_commit_author_date", Some(&git.head.date));

        if let Some(parent) = &git.parent {
            push("parent_commit_sha", Some(&parent.sha));
            push("parent_commit_author_name", Some(&parent.author_name));
            push("parent_commit_author_email", Some(&parent.author_email));
            push("parent_commit_author_message", Some(&parent.message));
            push("parent_commit_author_date", Some(&parent.date));
        }
    }

    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upload::git_meta::CommitMeta;

    fn head() -> CommitMeta {
        CommitMeta {
            sha: "deadbeef".to_string(),
            author_name: "Jer".to_string(),
            author_email: "jer@example.com".to_string(),
            message: "a commit".to_string(),
            date: "2026-08-20T00:00:00Z".to_string(),
        }
    }

    fn git(parent: Option<CommitMeta>) -> GitMetadata {
        GitMetadata {
            head: head(),
            parent,
            branch: Some("feat/x".to_string()),
            remote_url: Some("https://example.com/org/repo.git".to_string()),
            diff: "@@ -1 +1 @@\n-a\n+b".to_string(),
        }
    }

    fn ci() -> CiMetadata {
        CiMetadata {
            provider: "circleci",
            pr_number: Some("42".to_string()),
            branch: Some("feat/x".to_string()),
            build_num: Some("100".to_string()),
            job: Some("test".to_string()),
            username: Some("jrussell".to_string()),
        }
    }

    fn field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn maps_every_field_to_its_source() {
        let fields = build_fields(Some(&git(None)), &ci(), Some("rt"), Some("ot"));

        assert_eq!(field(&fields, "diff"), Some("@@ -1 +1 @@\n-a\n+b"));
        assert_eq!(field(&fields, "ci_provider"), Some("circleci"));
        assert_eq!(field(&fields, "ci_job"), Some("test"));
        assert_eq!(field(&fields, "ci_build"), Some("100"));
        assert_eq!(field(&fields, "ci_author"), Some("jrussell"));
        assert_eq!(field(&fields, "repo_token"), Some("rt"));
        assert_eq!(field(&fields, "org_token"), Some("ot"));
        assert_eq!(
            field(&fields, "git_repo"),
            Some("https://example.com/org/repo.git")
        );
        assert_eq!(field(&fields, "git_pr"), Some("42"));
        assert_eq!(field(&fields, "git_head_commit"), Some("deadbeef"));
        assert_eq!(field(&fields, "git_head_branch"), Some("feat/x"));
        assert_eq!(field(&fields, "git_branch"), Some("feat/x"));
        assert_eq!(field(&fields, "head_commit_author_name"), Some("Jer"));
        assert_eq!(field(&fields, "base_dir"), Some("."));
    }

    #[test]
    fn omits_parent_fields_when_there_is_no_parent_commit() {
        let fields = build_fields(Some(&git(None)), &ci(), Some("rt"), None);
        assert_eq!(field(&fields, "parent_commit_sha"), None);
    }

    #[test]
    fn includes_parent_fields_when_a_parent_commit_exists() {
        let mut parent = head();
        parent.sha = "parentsha".to_string();
        let fields = build_fields(Some(&git(Some(parent))), &ci(), Some("rt"), None);
        assert_eq!(field(&fields, "parent_commit_sha"), Some("parentsha"));
    }

    #[test]
    fn omits_org_token_when_not_provided() {
        let fields = build_fields(Some(&git(None)), &ci(), Some("rt"), None);
        assert_eq!(field(&fields, "org_token"), None);
    }

    #[test]
    fn git_base_branch_is_never_sent() {
        let fields = build_fields(Some(&git(None)), &ci(), Some("rt"), Some("ot"));
        assert_eq!(field(&fields, "git_base_branch"), None);
    }

    #[test]
    fn no_git_repo_omits_every_git_derived_field_but_keeps_ci_and_tokens() {
        let fields = build_fields(None, &ci(), Some("rt"), Some("ot"));

        for key in [
            "diff",
            "git_repo",
            "git_head_commit",
            "git_head_branch",
            "head_commit_author_name",
            "parent_commit_sha",
        ] {
            assert_eq!(field(&fields, key), None, "{key} should be absent");
        }
        assert_eq!(field(&fields, "ci_provider"), Some("circleci"));
        assert_eq!(field(&fields, "repo_token"), Some("rt"));
        assert_eq!(field(&fields, "git_pr"), Some("42"));
        assert_eq!(field(&fields, "base_dir"), Some("."));
    }

    fn response(status: u16, body: &str) -> RawResponse {
        RawResponse {
            status: reqwest::StatusCode::from_u16(status).expect("valid status"),
            body: body.to_string(),
        }
    }

    #[test]
    fn success_requires_both_a_2xx_status_and_the_marker() {
        assert!(is_success(&response(200, "Queued for processing")));
        assert!(!is_success(&response(500, "Queued for processing")));
        assert!(!is_success(&response(401, "unauthorized")));
        assert!(!is_success(&response(200, "unexpected body")));
    }
}
