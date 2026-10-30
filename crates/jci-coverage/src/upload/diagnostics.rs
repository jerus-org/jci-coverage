//! Safe, descriptive logging helpers for `upload`: what was provided and
//! sent, without ever revealing a secret.

use std::path::{Path, PathBuf};

/// How the upload token was supplied, without its value. The raw
/// `--repo-token` flag is `None` when the token comes from the environment
/// (the normal CI case), so the flag alone says nothing about whether a token
/// exists.
pub fn describe_token(flag: Option<&str>, env_name: &str, env_value: Option<&str>) -> String {
    let present = |v: Option<&str>| v.is_some_and(|v| !v.is_empty());
    if present(flag) {
        "provided via --repo-token (value withheld)".to_string()
    } else if present(env_value) {
        format!("provided via {env_name} (value withheld)")
    } else {
        "not provided".to_string()
    }
}

/// The multipart fields as they should appear in a log: the token is
/// withheld, the (large) diff is reduced to its size, long text is cut.
pub fn redact_fields(fields: &[(String, String)]) -> Vec<(String, String)> {
    const MAX_TEXT: usize = 80;
    fields
        .iter()
        .map(|(k, v)| {
            let shown = match k.as_str() {
                "repo_token" => "<redacted>".to_string(),
                "git_repo" => strip_userinfo(v),
                "diff" => format!("<{} bytes>", v.len()),
                _ if v.chars().count() > MAX_TEXT => {
                    let cut: String = v.chars().take(MAX_TEXT).collect();
                    format!("{cut}…")
                }
                _ => v.clone(),
            };
            (k.clone(), shown)
        })
        .collect()
}

/// A URL with any `user:password@` / `token@` part removed, so a remote such as
/// `https://x-access-token:ghp_XXXX@github.com/org/repo.git` is safe to log.
/// SCP-style remotes (`git@github.com:org/repo.git`) carry no secret and are
/// returned unchanged.
pub fn strip_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let (authority, tail) = match rest.find('/') {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    format!("{scheme}://{host}{tail}")
}

/// `path` made absolute against `cwd` (as `upload` will open it), with symlinks
/// resolved when the file exists.
pub fn absolute_path(path: &Path, cwd: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    std::fs::canonicalize(&joined).unwrap_or(joined)
}

/// One line saying which file is being uploaded: its absolute path, size and
/// how long ago it was written (a stale report from an earlier run shows here).
pub fn describe_file(path: &Path, cwd: &Path) -> String {
    let abs = absolute_path(path, cwd);
    match std::fs::metadata(&abs) {
        Ok(meta) => {
            let age = meta
                .modified()
                .ok()
                .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
                .map_or_else(
                    || "unknown".to_string(),
                    |d| format!("{}s ago", d.as_secs()),
                );
            format!("{} ({} bytes, modified {age})", abs.display(), meta.len())
        }
        Err(e) => format!("{} (cannot read: {e})", abs.display()),
    }
}

/// The error text for a coverage file that is not there: the absolute path that
/// was looked up, and what the directory holds instead — so `lcov.ino` sitting
/// beside a missing `lcov.info` (or a report written to another directory) is
/// visible at once.
pub fn file_not_found_message(path: &Path, cwd: &Path) -> String {
    const MAX_LISTED: usize = 20;
    let abs = absolute_path(path, cwd);
    let mut msg = format!(
        "coverage file not found: {} (looked in the working directory {})",
        abs.display(),
        cwd.display()
    );
    let Some(dir) = abs.parent() else {
        return msg;
    };
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            if names.is_empty() {
                msg.push_str(&format!("; the directory {} is empty", dir.display()));
            } else {
                let more = names.len().saturating_sub(MAX_LISTED);
                names.truncate(MAX_LISTED);
                msg.push_str(&format!(
                    "; the directory {} contains: {}{}",
                    dir.display(),
                    names.join(", "),
                    if more > 0 {
                        format!(" (and {more} more)")
                    } else {
                        String::new()
                    }
                ));
            }
        }
        Err(_) => msg.push_str(&format!("; the directory {} does not exist", dir.display())),
    }
    msg
}

/// What a coverage report says about its source paths, for checking them
/// against `base_dir`.
#[derive(Debug, PartialEq, Eq)]
pub struct ReportSummary {
    /// `SF:` records (lcov) or `<class filename>` attributes (Cobertura) in the
    /// report.
    pub source_files: usize,
    /// Of those, how many are absolute paths, as the report gives them.
    pub absolute: usize,
    /// Of those, how many sit under `base_dir`. A relative Cobertura path counts
    /// as under `base_dir` when the report's `<source>` root joined to it is.
    pub under_base_dir: usize,
    /// The first few source paths, verbatim.
    pub samples: Vec<String>,
    /// Why the scan of a Cobertura report stopped early, if it did: the counts
    /// above then cover only what was read.
    pub scan_error: Option<String>,
}

const MAX_SAMPLES: usize = 3;

impl ReportSummary {
    fn empty() -> Self {
        Self {
            source_files: 0,
            absolute: 0,
            under_base_dir: 0,
            samples: Vec::new(),
            scan_error: None,
        }
    }

    /// Record one source path. `root` is the directory a relative path is
    /// relative to, when the report names one.
    fn add(&mut self, path: &str, root: Option<&str>, base_dir: &str) {
        use std::path::Path;

        self.source_files += 1;
        let p = Path::new(path);
        if p.is_absolute() {
            self.absolute += 1;
        }
        let resolved = match root {
            Some(root) if !p.is_absolute() => Path::new(root).join(p),
            _ => p.to_path_buf(),
        };
        if resolved.starts_with(base_dir) {
            self.under_base_dir += 1;
        }
        if self.samples.len() < MAX_SAMPLES {
            self.samples.push(path.to_string());
        }
    }
}

/// Whether the report's absolute source paths all miss `base_dir`, which
/// suggests OtterWise cannot map them to repository files. Relative paths are
/// resolvable as they are, so a report with none is never a miss.
pub fn paths_miss_base_dir(summary: &ReportSummary) -> bool {
    summary.absolute > 0 && summary.under_base_dir == 0
}

/// Summarise the source paths of a coverage report, whether it is a Cobertura
/// XML report or an lcov one (told apart by the XML's opening `<`, after any
/// byte order mark).
pub fn summarize_report(content: &str, base_dir: &str) -> ReportSummary {
    let content = content.trim_start_matches('\u{feff}');
    if content.trim_start().starts_with('<') {
        summarize_cobertura(content, base_dir)
    } else {
        summarize_lcov(content, base_dir)
    }
}

/// The `filename` attribute of a `<class>`, or why its attributes are bad.
fn class_filename(e: &quick_xml::events::BytesStart<'_>) -> Result<Option<String>, String> {
    use quick_xml::XmlVersion;

    for attribute in e.attributes() {
        let attribute = attribute.map_err(|err| err.to_string())?;
        if attribute.key.as_ref() == "filename" {
            return attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map(|v| Some(v.into_owned()))
                .map_err(|err| err.to_string());
        }
    }
    Ok(None)
}

/// The report's first non-blank `<source>` text, found as its events are read.
#[derive(Default)]
struct SourceRoot {
    in_source: bool,
    root: Option<String>,
}

impl SourceRoot {
    fn track(&mut self, event: &quick_xml::events::Event<'_>) {
        use quick_xml::events::Event;

        match event {
            Event::Start(e) if e.name().as_ref() == "source" => self.in_source = true,
            Event::End(e) if e.name().as_ref() == "source" => self.in_source = false,
            Event::Text(t) if self.in_source && self.root.is_none() => {
                let text = t.as_ref().trim();
                if !text.is_empty() {
                    self.root = Some(text.to_string());
                }
            }
            _ => {}
        }
    }
}

/// Count the `<class filename>` attributes of a Cobertura report and how many
/// are under `base_dir`, resolving relative ones against the report's first
/// `<source>`. A malformed report ends the scan early and says why in
/// `scan_error`; it does not fail the upload.
fn summarize_cobertura(content: &str, base_dir: &str) -> ReportSummary {
    use quick_xml::{Reader, events::Event};

    let mut summary = ReportSummary::empty();
    let mut reader = Reader::from_str(content);
    let mut source = SourceRoot::default();
    loop {
        let event = match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(event) => event,
            Err(err) => {
                summary.scan_error = Some(err.to_string());
                break;
            }
        };
        source.track(&event);
        if let Event::Start(e) | Event::Empty(e) = &event
            && e.name().as_ref() == "class"
            && let Err(err) = add_class(&mut summary, e, source.root.as_deref(), base_dir)
        {
            summary.scan_error = Some(err);
            break;
        }
    }
    summary
}

/// Count the file a `<class>` names, if it names one.
fn add_class(
    summary: &mut ReportSummary,
    class: &quick_xml::events::BytesStart<'_>,
    root: Option<&str>,
    base_dir: &str,
) -> Result<(), String> {
    if let Some(path) = class_filename(class)? {
        summary.add(&path, root, base_dir);
    }
    Ok(())
}

/// Count the `SF:` records of an lcov report and how many are under `base_dir`.
pub fn summarize_lcov(content: &str, base_dir: &str) -> ReportSummary {
    let mut summary = ReportSummary::empty();
    for path in content.lines().filter_map(|l| l.strip_prefix("SF:")) {
        summary.add(path, None, base_dir);
    }
    summary
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn missing_file_message_shows_the_absolute_path_and_the_neighbouring_files() {
        let dir = tempdir().expect("tempdir");
        let coverage = dir.path().join("coverage");
        std::fs::create_dir(&coverage).expect("mkdir");
        std::fs::write(coverage.join("lcov.ino"), "").expect("write typo file");

        let msg = file_not_found_message(Path::new("coverage/lcov.info"), dir.path());

        let expected = coverage.join("lcov.info");
        assert!(msg.contains(&expected.display().to_string()), "{msg}");
        assert!(msg.contains("lcov.ino"), "{msg}");
    }

    #[test]
    fn missing_file_message_says_when_the_directory_does_not_exist() {
        let dir = tempdir().expect("tempdir");
        let msg = file_not_found_message(Path::new("coverage/lcov.info"), dir.path());
        assert!(msg.contains("does not exist"), "{msg}");
    }

    #[test]
    fn missing_file_message_says_when_the_directory_is_empty() {
        let dir = tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("coverage")).expect("mkdir");
        let msg = file_not_found_message(Path::new("coverage/lcov.info"), dir.path());
        assert!(msg.contains("is empty"), "{msg}");
    }

    #[test]
    fn describe_file_gives_the_absolute_path_size_and_age() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("lcov.info"), "TN:\n").expect("write");

        let d = describe_file(Path::new("lcov.info"), dir.path());

        let abs = std::fs::canonicalize(dir.path().join("lcov.info")).expect("canonical");
        assert!(d.contains(&abs.display().to_string()), "{d}");
        assert!(d.contains("4 bytes"), "{d}");
        assert!(d.contains("modified"), "{d}");
    }

    #[test]
    fn absolute_path_joins_relative_paths_to_the_working_directory() {
        assert_eq!(
            absolute_path(Path::new("coverage/lcov.info"), Path::new("/work")),
            Path::new("/work/coverage/lcov.info")
        );
        assert_eq!(
            absolute_path(Path::new("/abs/lcov.info"), Path::new("/work")),
            Path::new("/abs/lcov.info")
        );
    }

    fn kv(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn token_from_the_environment_is_described_without_its_value() {
        let d = describe_token(None, "OTTERWISE_TOKEN", Some("s3cret-value"));
        assert!(d.contains("OTTERWISE_TOKEN"), "{d}");
        assert!(d.contains("provided"), "{d}");
        assert!(d.contains("withheld"), "{d}");
        assert!(!d.contains("s3cret-value"), "{d}");
    }

    #[test]
    fn token_from_the_flag_wins_and_is_withheld() {
        let d = describe_token(Some("flag-secret"), "OTTERWISE_TOKEN", Some("env-secret"));
        assert!(d.contains("--repo-token"), "{d}");
        assert!(
            !d.contains("flag-secret") && !d.contains("env-secret"),
            "{d}"
        );
    }

    #[test]
    fn missing_or_empty_tokens_are_described_as_not_provided() {
        assert!(describe_token(None, "OTTERWISE_TOKEN", None).contains("not provided"));
        assert!(describe_token(Some(""), "OTTERWISE_TOKEN", Some("")).contains("not provided"));
    }

    #[test]
    fn redact_fields_withholds_the_token_and_sizes_the_diff() {
        let fields = vec![
            kv("repo_token", "s3cret-value"),
            kv("diff", "@@ -1 +1 @@\n-a\n+b"),
            kv("git_branch", "main"),
        ];
        let out = redact_fields(&fields);
        let get = |k: &str| out.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        assert_eq!(get("repo_token").as_deref(), Some("<redacted>"));
        assert_eq!(get("diff").as_deref(), Some("<17 bytes>"));
        assert_eq!(get("git_branch").as_deref(), Some("main"));
        assert!(!format!("{out:?}").contains("s3cret-value"));
    }

    #[test]
    fn redact_fields_truncates_long_text() {
        let long = "x".repeat(300);
        let out = redact_fields(&[kv("head_commit_author_message", &long)]);
        assert!(out[0].1.chars().count() < 120, "{}", out[0].1);
        assert!(out[0].1.ends_with('…'), "{}", out[0].1);
    }

    #[test]
    fn summarize_lcov_counts_sources_under_the_base_dir() {
        let lcov = "TN:\nSF:/work/proj/src/a.rs\nDA:1,1\nend_of_record\n\
                    SF:/work/proj/src/b.rs\nend_of_record\nSF:/elsewhere/c.rs\nend_of_record\n";
        let s = summarize_lcov(lcov, "/work/proj");
        assert_eq!(s.source_files, 3);
        assert_eq!(s.under_base_dir, 2);
        assert_eq!(s.samples[0], "/work/proj/src/a.rs");
        let none = summarize_lcov(lcov, ".");
        assert_eq!(
            none.under_base_dir, 0,
            "a relative base_dir matches nothing"
        );
    }

    #[test]
    fn summarize_lcov_counts_absolute_paths_separately_from_relative_ones() {
        let lcov = "SF:src/a.rs\nend_of_record\nSF:/work/proj/b.rs\nend_of_record\n";
        let s = summarize_lcov(lcov, "/work/proj");
        assert_eq!((s.source_files, s.absolute, s.under_base_dir), (2, 1, 1));
    }

    #[test]
    fn strip_userinfo_removes_embedded_credentials() {
        assert_eq!(
            strip_userinfo("https://x-access-token:ghp_SECRET@github.com/org/repo.git"),
            "https://github.com/org/repo.git"
        );
        assert_eq!(
            strip_userinfo("https://token@github.com/org/repo"),
            "https://github.com/org/repo"
        );
        let clean = "https://github.com/org/repo.git";
        assert_eq!(strip_userinfo(clean), clean);
        let scp = "git@github.com:org/repo.git";
        assert_eq!(strip_userinfo(scp), scp);
    }

    #[test]
    fn redact_fields_strips_credentials_from_the_repository_url() {
        let out = redact_fields(&[kv(
            "git_repo",
            "https://x-access-token:ghp_SECRET@github.com/org/repo.git",
        )]);
        assert_eq!(out[0].1, "https://github.com/org/repo.git");
    }

    #[test]
    fn summarize_lcov_keeps_at_most_three_samples() {
        let lcov: String = (0..10)
            .map(|i| format!("SF:/p/{i}.rs\nend_of_record\n"))
            .collect();
        assert_eq!(summarize_lcov(&lcov, "/p").samples.len(), 3);
    }

    const COBERTURA: &str = r#"<?xml version="1.0" ?>
<coverage><sources><source>/work/proj</source></sources><packages><package><classes>
<class filename="crates/x/src/a.rs" name="a"><methods/></class>
<class filename="crates/x/src/b.rs" name="b"/>
<class filename="/work/proj/crates/x/src/c.rs" name="c"/>
<class filename="/elsewhere/d.rs" name="d"/>
<class filename="crates/x/src/e.rs" name="e"/>
</classes></package></packages></coverage>"#;

    #[test]
    fn summarize_report_reads_the_class_files_of_a_cobertura_report() {
        let s = summarize_report(COBERTURA, "/work/proj");
        assert_eq!(s.source_files, 5);
        assert_eq!(s.absolute, 2);
        assert_eq!(
            s.under_base_dir, 4,
            "relative classes resolve against <source>, so only d.rs is outside base_dir"
        );
        assert_eq!(s.scan_error, None);
        assert_eq!(
            s.samples,
            vec![
                "crates/x/src/a.rs",
                "crates/x/src/b.rs",
                "/work/proj/crates/x/src/c.rs"
            ]
        );
    }

    #[test]
    fn summarize_report_still_reads_lcov() {
        let lcov = "SF:/work/proj/src/a.rs\nend_of_record\n";
        assert_eq!(
            summarize_report(lcov, "/work/proj"),
            summarize_lcov(lcov, "/work/proj")
        );
    }

    #[test]
    fn summarize_report_decodes_entities_and_survives_malformed_xml() {
        let xml = r#"<coverage><class filename="a&amp;b.rs"/><class filename="c.rs"#;
        let s = summarize_report(xml, "/work");
        assert_eq!(
            s.samples,
            vec!["a&b.rs"],
            "reads what it can before the break"
        );
        assert_eq!(summarize_report("<coverage/>", "/work").source_files, 0);
    }

    #[test]
    fn cobertura_class_paths_resolve_against_the_report_source_root() {
        let xml = r#"<coverage><sources><source>/elsewhere</source></sources>
<class filename="src/a.rs"/></coverage>"#;
        let s = summarize_report(xml, "/work/proj");
        assert_eq!((s.source_files, s.absolute, s.under_base_dir), (1, 0, 0));
        assert_eq!(s.samples, vec!["src/a.rs"], "samples stay verbatim");
        let no_source = summarize_report(
            r#"<coverage><class filename="src/a.rs"/></coverage>"#,
            "/work",
        );
        assert_eq!(
            no_source.under_base_dir, 0,
            "no <source>: nothing to resolve against"
        );
    }

    #[test]
    fn a_malformed_cobertura_report_records_why_the_scan_stopped() {
        let xml = r#"<coverage><class filename="a.rs"/><class filename="b.rs" bad="#;
        let s = summarize_report(xml, "/work");
        assert_eq!(s.source_files, 1);
        assert!(
            s.scan_error.as_deref().is_some_and(|e| !e.is_empty()),
            "{s:?}"
        );
        let bad_attr = summarize_report(
            r#"<coverage><class bad filename="a.rs"/></coverage>"#,
            "/work",
        );
        assert!(
            bad_attr.scan_error.is_some(),
            "a malformed attribute is reported, not skipped"
        );
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_a_cobertura_report() {
        let xml = format!("\u{feff}{COBERTURA}");
        assert_eq!(summarize_report(&xml, "/work/proj").source_files, 5);
    }

    #[test]
    fn the_base_dir_warning_needs_absolute_paths_that_all_miss_it() {
        // Mixed relative and absolute Cobertura paths with a source root under
        // base_dir: nothing to warn about.
        assert!(!paths_miss_base_dir(&summarize_report(
            COBERTURA,
            "/work/proj"
        )));
        // lcov with only absolute paths outside base_dir: warn.
        let lcov = "SF:/elsewhere/a.rs\nend_of_record\n";
        assert!(paths_miss_base_dir(&summarize_report(lcov, "/work/proj")));
        // Relative paths only: resolvable as they are.
        let rel = "SF:src/a.rs\nend_of_record\n";
        assert!(!paths_miss_base_dir(&summarize_report(rel, "/work/proj")));
    }

    #[test]
    fn an_undecodable_class_filename_is_recorded_as_a_scan_error() {
        let xml = r#"<coverage><class filename="a&bogus;b.rs"/></coverage>"#;
        let s = summarize_report(xml, "/work");
        assert_eq!(s.source_files, 0);
        assert!(s.scan_error.is_some(), "{s:?}");
    }

    #[test]
    fn the_first_non_blank_source_is_the_root_for_relative_classes() {
        let xml = r#"<coverage><sources><source>  </source><source> /work/proj </source>
<source>/elsewhere</source></sources><class filename="src/a.rs"/></coverage>"#;
        let s = summarize_report(xml, "/work/proj");
        assert_eq!(
            s.under_base_dir, 1,
            "a blank <source> is skipped, the first real one wins and is trimmed"
        );
    }

    #[test]
    fn a_class_before_the_source_has_no_root_to_resolve_against() {
        let xml = r#"<coverage><class filename="src/a.rs"/>
<sources><source>/work/proj</source></sources><class filename="src/b.rs"/></coverage>"#;
        let s = summarize_report(xml, "/work/proj");
        assert_eq!((s.source_files, s.under_base_dir), (2, 1));
    }

    #[test]
    fn text_outside_a_source_is_not_taken_as_the_root() {
        let xml = r#"<coverage>/work/proj<sources><source/></sources>
<class filename="src/a.rs"/></coverage>"#;
        let s = summarize_report(xml, "/work/proj");
        assert_eq!(s.under_base_dir, 0);
    }

    #[test]
    fn text_after_the_source_closes_is_not_taken_as_the_root() {
        let xml = r#"<coverage><sources><source></source>/work/proj</sources>
<class filename="src/a.rs"/></coverage>"#;
        let s = summarize_report(xml, "/work/proj");
        assert_eq!(s.under_base_dir, 0);
    }

    #[test]
    fn the_scan_stops_at_a_class_with_bad_attributes() {
        let xml = r#"<coverage><class filename="a.rs"/><class bad filename="b.rs"/>
<class filename="c.rs"/></coverage>"#;
        let s = summarize_report(xml, "/work");
        assert_eq!(s.source_files, 1, "classes after the error are not counted");
        assert!(s.scan_error.is_some(), "{s:?}");
    }

    #[test]
    fn a_class_without_a_filename_is_skipped_without_an_error() {
        let xml = r#"<coverage><class name="x"/><class filename="a.rs"/></coverage>"#;
        let s = summarize_report(xml, "/work");
        assert_eq!(s.source_files, 1);
        assert_eq!(s.scan_error, None);
    }
}
