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

/// `owner/repo` from a github.com remote URL (`git@github.com:o/r.git`,
/// `https://github.com/o/r`), the form OtterWise's own uploader sends as
/// `git_repo`. `None` for any other host or shape: better omitted than sent as
/// a URL, which would also carry any credentials embedded in it.
fn repo_slug(url: &str) -> Option<String> {
    let (host, path) = match url.split_once("://") {
        // `scheme://[user[:pass]@]host[:port]/owner/repo`
        Some((_, rest)) => rest.split_once('/')?,
        // scp-like `[user@]host:owner/repo`
        None => url.split_once(':')?,
    };
    let host = host.rsplit('@').next()?.split(':').next()?;
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    (!owner.is_empty() && !name.is_empty() && !name.contains('/'))
        .then(|| format!("{owner}/{name}"))
}

/// `git` is `None` when `upload` is run outside a git repo (or one with no
/// commits yet) — `upload`'s own docs (see `cli.rs`) call it standalone, so
/// that's not a failure: every git-derived field below is simply omitted.
///
/// `git_base_branch` (the PR's merge target) has no CircleCI env-var
/// equivalent, unlike every other field here — CircleCI doesn't expose it.
/// Left unsent; OtterWise falls back to full-file coverage instead of diff
/// coverage until this is filled in. See ROADMAP.md.
///
/// `base_dir` is the absolute project directory. Coverage tools such as
/// `cargo llvm-cov` write absolute source paths, and OtterWise uses this to map
/// them back to repository files (its own uploader defaults it to the working
/// directory); a relative `.` would strip nothing.
pub fn build_fields(
    git: Option<&GitMetadata>,
    ci: &CiMetadata,
    repo_token: Option<&str>,
    base_dir: &str,
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
    push("git_pr", ci.pr_number.as_deref());
    push("git_branch", ci.branch.as_deref());
    push("base_dir", Some(base_dir));

    if let Some(git) = git {
        push("diff", Some(&git.diff));
        let repo = ci
            .repo
            .clone()
            .or_else(|| git.remote_url.as_deref().and_then(repo_slug));
        push("git_repo", repo.as_deref());
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
    use std::collections::BTreeSet;

    use super::*;
    use crate::upload::git_meta::CommitMeta;

    /// Form fields OtterWise's own bash uploader posts, recorded in a file the
    /// `scripts/check-otterwise-drift.sh` check keeps in step with the live script.
    const REFERENCE_FIELDS: &str = include_str!("../../tests/otterwise_reference_fields.txt");

    /// Reference fields jci-coverage deliberately does not send, and why. A new
    /// divergence needs an entry here; a closed one must be removed.
    const KNOWN_UNSENT: &[(&str, &str)] = &[
        ("org_token", "org tokens dropped deliberately (8fba9de)"),
        (
            "git_base_branch",
            "CircleCI exposes no PR base branch (ROADMAP)",
        ),
        ("log_file", "junit log upload not supported"),
        ("config_file", ".otterwise.yml upload not supported"),
        ("mutation_file", "mutation reports not supported"),
        ("type_coverage_file", "type coverage not supported"),
        ("type_coverage_format", "type coverage not supported"),
        ("flag", "flags not supported"),
        ("component", "components not supported"),
        ("part", "split uploads not supported"),
        ("part_total", "split uploads not supported"),
    ];

    fn reference() -> BTreeSet<&'static str> {
        REFERENCE_FIELDS
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    }

    /// Every field jci-coverage can send: all inputs present, plus the report file.
    fn sent_fields() -> BTreeSet<String> {
        let mut parent = head();
        parent.sha = "parentsha".to_string();
        let fields = build_fields(Some(&git(Some(parent))), &ci(), Some("rt"), "/work/proj");
        let mut sent: BTreeSet<String> = fields.into_iter().map(|(k, _)| k).collect();
        sent.insert(crate::upload::FILE_FIELD.to_string());
        sent
    }

    #[test]
    fn reference_fields_we_send_are_all_fields_the_reference_uploader_posts() {
        let reference = reference();
        let invented: Vec<_> = sent_fields()
            .into_iter()
            .filter(|f| !reference.contains(f.as_str()))
            .collect();
        assert!(
            invented.is_empty(),
            "fields the reference uploader never sends: {invented:?}"
        );
    }

    #[test]
    fn reference_fields_we_do_not_send_are_exactly_the_documented_divergences() {
        let sent = sent_fields();
        let unsent: BTreeSet<_> = reference()
            .into_iter()
            .filter(|f| !sent.contains(*f))
            .collect();
        let documented: BTreeSet<_> = KNOWN_UNSENT.iter().map(|(f, _)| *f).collect();
        assert_eq!(
            unsent, documented,
            "reference fields we skip must match KNOWN_UNSENT (add the reason, or remove the entry once sent)"
        );
    }

    fn head() -> CommitMeta {
        CommitMeta {
            sha: "deadbeef".to_string(),
            author_name: "Jer".to_string(),
            author_email: "jer@example.com".to_string(),
            message: "a commit".to_string(),
            date: "1787184000".to_string(),
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
            provider: "circle-ci",
            pr_number: Some("42".to_string()),
            branch: Some("feat/x".to_string()),
            build_num: Some("100".to_string()),
            job: Some("test".to_string()),
            username: Some("jrussell".to_string()),
            repo: Some("org/repo".to_string()),
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
        let fields = build_fields(Some(&git(None)), &ci(), Some("rt"), "/work/proj");

        assert_eq!(field(&fields, "diff"), Some("@@ -1 +1 @@\n-a\n+b"));
        assert_eq!(field(&fields, "ci_provider"), Some("circle-ci"));
        assert_eq!(field(&fields, "ci_job"), Some("test"));
        assert_eq!(field(&fields, "ci_build"), Some("100"));
        assert_eq!(field(&fields, "ci_author"), Some("jrussell"));
        assert_eq!(field(&fields, "repo_token"), Some("rt"));
        assert_eq!(field(&fields, "git_repo"), Some("org/repo"));
        assert_eq!(field(&fields, "git_pr"), Some("42"));
        assert_eq!(field(&fields, "git_head_commit"), Some("deadbeef"));
        assert_eq!(field(&fields, "git_head_branch"), Some("feat/x"));
        assert_eq!(field(&fields, "git_branch"), Some("feat/x"));
        assert_eq!(field(&fields, "head_commit_author_name"), Some("Jer"));
        assert_eq!(field(&fields, "base_dir"), Some("/work/proj"));
    }

    #[test]
    fn git_repo_falls_back_to_the_slug_of_the_remote_url() {
        let mut ci = ci();
        ci.repo = None;
        let mut g = git(None);
        g.remote_url = Some("git@github.com:jerus-org/jci-coverage.git".to_string());
        let fields = build_fields(Some(&g), &ci, Some("rt"), "/work/proj");
        assert_eq!(field(&fields, "git_repo"), Some("jerus-org/jci-coverage"));
    }

    #[test]
    fn git_repo_prefers_the_ci_slug_over_the_remote_url() {
        let mut g = git(None);
        g.remote_url = Some("git@github.com:someone-else/fork.git".to_string());
        let fields = build_fields(Some(&g), &ci(), Some("rt"), "/work/proj");
        assert_eq!(field(&fields, "git_repo"), Some("org/repo"));
    }

    #[test]
    fn git_repo_is_omitted_rather_than_sent_as_a_raw_url() {
        let mut ci = ci();
        ci.repo = None;
        let mut g = git(None);
        g.remote_url = Some("https://user:secret@example.com/org/repo.git".to_string());
        let fields = build_fields(Some(&g), &ci, Some("rt"), "/work/proj");
        assert_eq!(field(&fields, "git_repo"), None);
    }

    #[test]
    fn repo_slug_handles_the_common_github_url_forms() {
        for url in [
            "git@github.com:o/r.git",
            "https://github.com/o/r.git",
            "https://github.com/o/r",
            "https://x-access-token:tok@github.com/o/r.git",
            "ssh://git@github.com/o/r.git",
        ] {
            assert_eq!(repo_slug(url).as_deref(), Some("o/r"), "{url}");
        }
        assert_eq!(repo_slug("https://github.com/o"), None);
        assert_eq!(repo_slug("https://notgithub.com/o/r"), None);
        assert_eq!(repo_slug("https://github.com.evil.example/o/r"), None);
        assert_eq!(repo_slug("https://gitlab.com/o/r.git"), None);
        assert_eq!(repo_slug("https://example.com/o/r"), None);
    }

    #[test]
    fn omits_parent_fields_when_there_is_no_parent_commit() {
        let fields = build_fields(Some(&git(None)), &ci(), Some("rt"), "/work/proj");
        assert_eq!(field(&fields, "parent_commit_sha"), None);
    }

    #[test]
    fn includes_parent_fields_when_a_parent_commit_exists() {
        let mut parent = head();
        parent.sha = "parentsha".to_string();
        let fields = build_fields(Some(&git(Some(parent))), &ci(), Some("rt"), "/work/proj");
        assert_eq!(field(&fields, "parent_commit_sha"), Some("parentsha"));
    }

    #[test]
    fn git_base_branch_is_never_sent() {
        let fields = build_fields(Some(&git(None)), &ci(), Some("rt"), "/work/proj");
        assert_eq!(field(&fields, "git_base_branch"), None);
    }

    #[test]
    fn no_git_repo_omits_every_git_derived_field_but_keeps_ci_and_tokens() {
        let fields = build_fields(None, &ci(), Some("rt"), "/work/proj");

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
        assert_eq!(field(&fields, "ci_provider"), Some("circle-ci"));
        assert_eq!(field(&fields, "repo_token"), Some("rt"));
        assert_eq!(field(&fields, "git_pr"), Some("42"));
        assert_eq!(field(&fields, "base_dir"), Some("/work/proj"));
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
