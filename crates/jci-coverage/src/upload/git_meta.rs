//! Reads the git metadata OtterWise needs from the already-checked-out repo:
//! HEAD and parent commit details, current branch, and the stripped diff
//! between them. Uses `git2` directly rather than shelling to `git`, so
//! output parsing isn't a factor.

use std::path::Path;

use anyhow::{Context, Result};
use git2::{Commit, Repository};

use super::diff;

/// Author/committer detail for one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMeta {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    pub message: String,
    pub date: String,
}

/// Everything `upload` reads from the local repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitMetadata {
    pub head: CommitMeta,
    /// `None` for a repo's first commit, which has no parent to diff against.
    pub parent: Option<CommitMeta>,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub diff: String,
}

/// RFC3339, matching what the reference bash uploader sends (`git log
/// --format=%aI`).
fn format_time(t: git2::Time) -> String {
    let offset = time::UtcOffset::from_whole_seconds(t.offset_minutes() * 60)
        .unwrap_or(time::UtcOffset::UTC);
    match time::OffsetDateTime::from_unix_timestamp(t.seconds()) {
        Ok(utc) => utc
            .to_offset(offset)
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        Err(_) => String::new(),
    }
}

fn commit_meta(commit: &Commit) -> CommitMeta {
    let author = commit.author();
    CommitMeta {
        sha: commit.id().to_string(),
        author_name: author.name().unwrap_or_default().to_string(),
        author_email: author.email().unwrap_or_default().to_string(),
        message: commit
            .summary()
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_string(),
        date: format_time(author.when()),
    }
}

/// Unified, zero-context diff between two trees, stripped for OtterWise.
/// Only called when a parent commit exists — a first commit has nothing to
/// diff against and its `diff` field is simply empty (see [`collect`]).
fn stripped_diff(repo: &Repository, base: &Commit, head: &Commit) -> Result<String> {
    let base_tree = base.tree()?;
    let head_tree = head.tree()?;
    let mut opts = git2::DiffOptions::new();
    opts.context_lines(0);
    let git_diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut opts))?;

    let mut raw = String::new();
    git_diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        if !matches!(
            line.origin(),
            '+' | '-' | ' ' | 'F' | 'H' | 'B' | '<' | '>' | '='
        ) {
            return true;
        }
        let prefix = match line.origin() {
            '+' | '-' | ' ' => line.origin().to_string(),
            _ => String::new(),
        };
        raw.push_str(&prefix);
        raw.push_str(&String::from_utf8_lossy(line.content()));
        true
    })?;

    Ok(diff::strip(&raw))
}

/// Read HEAD/parent commit metadata, branch, remote, and the stripped diff
/// between them from the repo containing `path`.
///
/// `upload` is documented as standalone (works on any coverage file, no
/// Rust/git assumption) — so the *absence* of a git repo, or a repo with no
/// commits yet, is not a failure here, just `Ok(None)`: those callers get
/// every field except the git-derived ones. A genuine git2 error (a corrupt
/// object, an unreadable tree) still propagates as `Err`.
pub fn collect(path: &Path) -> Result<Option<GitMetadata>> {
    let Ok(repo) = Repository::discover(path) else {
        return Ok(None);
    };
    let Ok(head_ref) = repo.head() else {
        return Ok(None);
    };
    let head_commit = head_ref
        .peel_to_commit()
        .context("HEAD does not point at a commit")?;
    let parent_commit = head_commit.parent(0).ok();

    let diff = match &parent_commit {
        Some(parent) => stripped_diff(&repo, parent, &head_commit)?,
        None => String::new(),
    };

    let branch = head_ref.shorthand().ok().map(str::to_string);
    let remote_url = repo
        .find_remote("origin")
        .ok()
        .and_then(|r| r.url().ok().map(str::to_string));

    Ok(Some(GitMetadata {
        head: commit_meta(&head_commit),
        parent: parent_commit.as_ref().map(commit_meta),
        branch,
        remote_url,
        diff,
    }))
}

#[cfg(test)]
mod tests {
    use git2::Oid;
    use tempfile::tempdir;

    use super::*;

    /// A repo with one commit, an `origin` remote, and no parent — the
    /// minimum shape `collect` must handle.
    fn init_repo_one_commit() -> (tempfile::TempDir, Oid) {
        let dir = tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("init");
        repo.remote("origin", "https://example.com/org/repo.git")
            .expect("add remote");
        std::fs::write(dir.path().join("a.txt"), "one\n").expect("write");

        let sig = git2::Signature::now("Test Author", "author@example.com").expect("sig");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("a.txt")).expect("add");
        let tree_id = index.write_tree().expect("write_tree");
        let tree = repo.find_tree(tree_id).expect("find_tree");
        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, "first commit", &tree, &[])
            .expect("commit");
        (dir, oid)
    }

    fn add_second_commit(dir: &Path) -> Oid {
        let repo = Repository::open(dir).expect("open");
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").expect("write");
        let sig = git2::Signature::now("Test Author", "author@example.com").expect("sig");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("a.txt")).expect("add");
        let tree_id = index.write_tree().expect("write_tree");
        let tree = repo.find_tree(tree_id).expect("find_tree");
        let parent = repo.head().expect("head").peel_to_commit().expect("commit");
        repo.commit(Some("HEAD"), &sig, &sig, "second commit", &tree, &[&parent])
            .expect("commit")
    }

    #[test]
    fn first_commit_has_no_parent_and_an_empty_diff() {
        let (dir, oid) = init_repo_one_commit();
        let meta = collect(dir.path()).expect("collect").expect("in a repo");

        assert_eq!(meta.head.sha, oid.to_string());
        assert_eq!(meta.head.author_name, "Test Author");
        assert_eq!(meta.head.message, "first commit");
        assert!(meta.parent.is_none());
        assert_eq!(meta.diff, "", "a first commit has nothing to diff against");
        assert_eq!(
            meta.remote_url.as_deref(),
            Some("https://example.com/org/repo.git")
        );
    }

    #[test]
    fn second_commit_has_a_parent_and_a_diff_against_it() {
        let (dir, first_oid) = init_repo_one_commit();
        let second_oid = add_second_commit(dir.path());
        let meta = collect(dir.path()).expect("collect").expect("in a repo");

        assert_eq!(meta.head.sha, second_oid.to_string());
        let parent = meta.parent.expect("has a parent");
        assert_eq!(parent.sha, first_oid.to_string());
        assert!(meta.diff.contains("+two"), "diff was: {:?}", meta.diff);
        assert!(
            !meta.diff.contains("diff --git"),
            "diff was: {:?}",
            meta.diff
        );
    }

    #[test]
    fn branch_shorthand_is_read_from_head() {
        let (dir, _) = init_repo_one_commit();
        let meta = collect(dir.path()).expect("collect").expect("in a repo");
        // git2::Repository::init defaults to "master" unless configured
        // otherwise; either default is fine, just confirm it's populated.
        assert!(meta.branch.is_some());
    }

    #[test]
    fn outside_any_git_repo_returns_none_not_an_error() {
        let dir = tempdir().expect("tempdir");
        // No git2::Repository::init: plain directory, no `.git` anywhere
        // above it either (tempdir roots are never inside a repo).
        let meta = collect(dir.path()).expect("not an error");
        assert!(meta.is_none());
    }

    #[test]
    fn a_repo_with_zero_commits_returns_none_not_an_error() {
        let dir = tempdir().expect("tempdir");
        Repository::init(dir.path()).expect("init");
        let meta = collect(dir.path()).expect("not an error");
        assert!(meta.is_none());
    }
}
