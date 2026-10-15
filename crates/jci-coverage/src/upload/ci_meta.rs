//! Reads the CircleCI-specific fields OtterWise wants. Takes an env lookup
//! rather than calling `std::env::var` directly, so tests don't depend on
//! real process env vars.

/// CircleCI-specific fields for the OtterWise payload. `provider` is
/// hardcoded: only CircleCI is supported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiMetadata {
    pub provider: &'static str,
    pub pr_number: Option<String>,
    pub branch: Option<String>,
    pub build_num: Option<String>,
    pub job: Option<String>,
    pub username: Option<String>,
    /// `owner/repo`, from `CIRCLE_PROJECT_USERNAME`/`CIRCLE_PROJECT_REPONAME`.
    pub repo: Option<String>,
}

/// `CIRCLE_PR_NUMBER` is only set for forked-repo PRs; same-repo PRs (this
/// org's own workflow — see the garden's branch-workflow skill) only get
/// `CIRCLE_PULL_REQUEST`, a full URL like
/// `https://github.com/org/repo/pull/42`. Try the number var first, then
/// fall back to the URL's trailing numeric segment.
fn pr_number(get_env: &impl Fn(&str) -> Option<String>) -> Option<String> {
    get_env("CIRCLE_PR_NUMBER").or_else(|| {
        get_env("CIRCLE_PULL_REQUEST").and_then(|url| {
            let segment = url.rsplit('/').next()?;
            (!segment.is_empty() && segment.chars().all(|c| c.is_ascii_digit()))
                .then(|| segment.to_string())
        })
    })
}

fn repo_slug(get_env: &impl Fn(&str) -> Option<String>) -> Option<String> {
    let owner = get_env("CIRCLE_PROJECT_USERNAME").filter(|v| !v.is_empty())?;
    let name = get_env("CIRCLE_PROJECT_REPONAME").filter(|v| !v.is_empty())?;
    Some(format!("{owner}/{name}"))
}

pub fn collect(get_env: impl Fn(&str) -> Option<String>) -> CiMetadata {
    CiMetadata {
        provider: "circle-ci",
        pr_number: pr_number(&get_env),
        branch: get_env("CIRCLE_BRANCH"),
        build_num: get_env("CIRCLE_BUILD_NUM"),
        job: get_env("CIRCLE_JOB"),
        username: get_env("CIRCLE_USERNAME"),
        repo: repo_slug(&get_env),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn reads_every_field_from_the_matching_circle_var() {
        let meta = collect(lookup(&[
            ("CIRCLE_PR_NUMBER", "42"),
            ("CIRCLE_BRANCH", "feat/x"),
            ("CIRCLE_BUILD_NUM", "100"),
            ("CIRCLE_JOB", "test"),
            ("CIRCLE_USERNAME", "jrussell"),
            ("CIRCLE_PROJECT_USERNAME", "jerus-org"),
            ("CIRCLE_PROJECT_REPONAME", "jci-coverage"),
        ]));
        assert_eq!(meta.repo.as_deref(), Some("jerus-org/jci-coverage"));
        assert_eq!(meta.provider, "circle-ci");
        assert_eq!(meta.pr_number.as_deref(), Some("42"));
        assert_eq!(meta.branch.as_deref(), Some("feat/x"));
        assert_eq!(meta.build_num.as_deref(), Some("100"));
        assert_eq!(meta.job.as_deref(), Some("test"));
        assert_eq!(meta.username.as_deref(), Some("jrussell"));
    }

    #[test]
    fn missing_vars_become_none_not_empty_string() {
        let meta = collect(lookup(&[]));
        assert_eq!(meta.pr_number, None);
        assert_eq!(meta.branch, None);
        assert_eq!(meta.build_num, None);
        assert_eq!(meta.job, None);
        assert_eq!(meta.username, None);
        assert_eq!(meta.repo, None);
    }

    #[test]
    fn repo_needs_both_the_owner_and_the_name() {
        let meta = collect(lookup(&[("CIRCLE_PROJECT_USERNAME", "jerus-org")]));
        assert_eq!(meta.repo, None);
    }

    #[test]
    fn pr_number_prefers_circle_pr_number_when_both_are_set() {
        let meta = collect(lookup(&[
            ("CIRCLE_PR_NUMBER", "7"),
            ("CIRCLE_PULL_REQUEST", "https://github.com/org/repo/pull/99"),
        ]));
        assert_eq!(meta.pr_number.as_deref(), Some("7"));
    }

    #[test]
    fn pr_number_falls_back_to_the_pull_request_url_for_same_repo_prs() {
        let meta = collect(lookup(&[(
            "CIRCLE_PULL_REQUEST",
            "https://github.com/jerus-org/jci-coverage/pull/42",
        )]));
        assert_eq!(meta.pr_number.as_deref(), Some("42"));
    }

    #[test]
    fn pr_number_is_none_when_the_url_has_no_trailing_number() {
        let meta = collect(lookup(&[(
            "CIRCLE_PULL_REQUEST",
            "https://example.com/pull/",
        )]));
        assert_eq!(meta.pr_number, None);
    }
}
