//! An lcov report as the input of `complexity`.
//!
//! `cargo llvm-cov --cobertura` is `llvm-cov export -format=lcov` followed by a
//! conversion to Cobertura with the `lcov2cobertura` crate. Converting the lcov
//! report the same way here gives the report `cargo llvm-cov` would have
//! written, so everything after it (matching methods to source, async futures,
//! closures, duplicates, the gate) is unchanged.
//!
//! An lcov report names its files by the absolute path they had where it was
//! written, which is often not where the checkout is. [`base_dir`] finds the
//! directory to strip from them, by looking for the source files under the
//! source root.

use std::{
    collections::BTreeMap,
    io::BufRead,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

/// The format of a report given to `complexity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Format {
    Cobertura,
    Lcov,
}

/// Which format `text` is, from its first non-blank line.
pub(super) fn detect(text: &str) -> Result<Format> {
    let first = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty());
    match first {
        Some(line) if line.starts_with('<') => Ok(Format::Cobertura),
        Some(line) if line.starts_with("TN:") || line.starts_with("SF:") => Ok(Format::Lcov),
        _ => bail!(
            "it is neither a Cobertura report (XML, starting with `<`) nor an lcov report \
             (starting with `TN:` or `SF:`)"
        ),
    }
}

/// Convert an lcov report to a Cobertura report, finding its source files under
/// `source_root`.
pub(super) fn to_cobertura(lcov: &str, source_root: &Path) -> Result<String> {
    if !lcov.lines().any(|line| line.starts_with("FN:")) {
        bail!(
            "the lcov report has no function records (`FN:` lines); `complexity` matches \
             functions to the source with them. `cargo llvm-cov --lcov` writes them; lcov from \
             another tool works if it has them and names the functions as Rust paths \
             (`crate::module::function`)"
        );
    }
    let files: Vec<&str> = lcov
        .lines()
        .filter_map(|line| line.strip_prefix("SF:"))
        .collect();
    let base = base_dir(&files, source_root);
    let data = lcov2cobertura::parse_lines(lcov.as_bytes().lines(), &base, &[])
        .context("failed to convert the lcov report to Cobertura")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    lcov2cobertura::coverage_to_string(&data, now, lcov2cobertura::RustDemangler::new())
        .context("failed to write the converted lcov report")
}

/// The directory to strip from the report's file names: the one that most of
/// them share and under which their tails exist beneath `source_root`.
///
/// Each absolute name proposes the directory left when the longest tail of it
/// that exists under `source_root` is taken off; the proposal made by most of
/// the names wins, and the smallest on a tie. A name that does not start with
/// the winner (a dependency's source, say) is left as it is. Nothing found is
/// the empty path, which strips nothing.
pub(super) fn base_dir(files: &[&str], source_root: &Path) -> PathBuf {
    let mut proposed: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for file in files {
        if let Some(dir) = proposal(Path::new(file), source_root) {
            *proposed.entry(dir).or_default() += 1;
        }
    }
    let mut best: Option<(PathBuf, usize)> = None;
    for (dir, count) in proposed {
        if best.as_ref().is_none_or(|(_, most)| count > *most) {
            best = Some((dir, count));
        }
    }
    best.map(|(dir, _)| dir).unwrap_or_default()
}

/// The directory `file` has on the machine that wrote the report, taken from the
/// longest tail of it that is a file under `source_root`.
fn proposal(file: &Path, source_root: &Path) -> Option<PathBuf> {
    if !file.is_absolute() {
        return None;
    }
    let parts: Vec<Component<'_>> = file.components().collect();
    (1..parts.len()).find_map(|split| {
        let tail: PathBuf = parts[split..].iter().collect();
        source_root
            .join(tail)
            .is_file()
            .then(|| parts[..split].iter().collect())
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::complexity::cobertura;

    /// Two functions of one file, as `cargo llvm-cov --lcov` writes them (v0
    /// mangled names), `add` run three times and `mult` never.
    fn lcov(file: &str) -> String {
        format!(
            "TN:\nSF:{file}\n\
             FN:1,_RNvCs2mwpCrQKxwN_4demo3add\n\
             FN:5,_RNvCs2mwpCrQKxwN_4demo4mult\n\
             FNDA:3,_RNvCs2mwpCrQKxwN_4demo3add\n\
             FNDA:0,_RNvCs2mwpCrQKxwN_4demo4mult\n\
             FNF:2\nFNH:1\n\
             DA:1,3\nDA:2,3\nDA:5,0\n\
             LF:3\nLH:2\nend_of_record\n"
        )
    }

    /// A source root holding the given files, each with the same content.
    fn root_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        for f in files {
            let path = dir.path().join(f);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(&path, "fn add() {}\n").expect("write");
        }
        dir
    }

    #[test]
    fn a_cobertura_report_is_told_by_its_first_tag() {
        for text in [
            "<?xml version=\"1.0\" ?>\n<coverage/>",
            "<coverage/>",
            "\n  \n<!DOCTYPE coverage>",
            "\u{feff}<?xml version=\"1.0\"?>",
        ] {
            assert_eq!(detect(text).expect(text), Format::Cobertura, "{text:?}");
        }
    }

    #[test]
    fn an_lcov_report_is_told_by_its_first_record() {
        for text in ["TN:\nSF:/a.rs\n", "SF:/a.rs\n", "\n\nTN:name\n"] {
            assert_eq!(detect(text).expect(text), Format::Lcov, "{text:?}");
        }
    }

    #[test]
    fn anything_else_is_refused_naming_both_formats() {
        for text in ["", "   \n", "{\"data\": []}", "hello", "FN:1,x\n"] {
            let err = detect(text).expect_err(text).to_string();
            assert!(
                err.contains("Cobertura") && err.contains("lcov"),
                "{text:?}: {err}"
            );
        }
    }

    #[test]
    fn the_converted_report_is_one_the_cobertura_parser_reads() {
        let root = root_with(&["src/lib.rs"]);
        let xml = to_cobertura(&lcov("/build/demo/src/lib.rs"), root.path()).expect("converts");
        let report = cobertura::parse(&xml).expect("parses");
        let names: Vec<&str> = report.methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["demo::add", "demo::mult"]);
        assert_eq!(report.classes.len(), 1);
        assert_eq!(report.classes[0].filename, "src/lib.rs");
        report
            .require_rust_llvm_cov()
            .expect("a Rust llvm-cov report");
    }

    #[test]
    fn the_files_are_found_under_the_source_root_wherever_the_report_was_written() {
        // Written at /build/demo, checked out under the source root.
        let root = root_with(&["crates/a/src/lib.rs", "crates/b/src/lib.rs"]);
        let files = [
            "/build/demo/crates/a/src/lib.rs",
            "/build/demo/crates/b/src/lib.rs",
        ];
        assert_eq!(base_dir(&files, root.path()), PathBuf::from("/build/demo"));
    }

    #[test]
    fn the_base_is_the_directory_most_files_share_and_a_stray_file_stays_absolute() {
        let root = root_with(&["src/lib.rs", "src/x.rs", "src/y.rs"]);
        // `/reg/foo/src/lib.rs` has the same tail as a file of the project, but
        // only one file shares its directory.
        let files = [
            "/build/demo/src/lib.rs",
            "/build/demo/src/x.rs",
            "/build/demo/src/y.rs",
            "/reg/foo/src/lib.rs",
        ];
        assert_eq!(base_dir(&files, root.path()), PathBuf::from("/build/demo"));

        let text = format!(
            "{}{}",
            lcov("/build/demo/src/lib.rs"),
            lcov("/reg/foo/src/lib.rs")
        );
        let xml = to_cobertura(&text, root.path()).expect("converts");
        let report = cobertura::parse(&xml).expect("parses");
        let mut names: Vec<&str> = report.classes.iter().map(|c| c.filename.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["/reg/foo/src/lib.rs", "src/lib.rs"]);
    }

    /// What a report holds, whatever the order `lcov2cobertura` wrote its classes
    /// in: each class's lines and each method, sorted.
    #[derive(Debug, PartialEq, Eq)]
    struct Contents {
        /// File and its (line, hits) pairs.
        classes: Vec<(String, Vec<(u32, u64)>)>,
        /// File, name, start line and hits.
        methods: Vec<(String, String, u32, u64)>,
    }

    fn contents(xml: &str) -> Contents {
        let report = cobertura::parse(xml).expect("parses");
        let mut classes: Vec<_> = report
            .classes
            .iter()
            .map(|c| {
                let mut lines = c.lines.clone();
                lines.sort_unstable();
                (c.filename.clone(), lines)
            })
            .collect();
        classes.sort();
        let mut methods: Vec<_> = report
            .methods
            .iter()
            .map(|m| {
                let file = report.classes[m.class].filename.clone();
                (file, m.name.clone(), m.line, m.hits)
            })
            .collect();
        methods.sort();
        Contents { classes, methods }
    }

    #[test]
    fn converting_an_lcov_report_gives_the_report_cargo_llvm_cov_writes() {
        // The same code (generics, an async fn and its future, closures, a test),
        // run once as `cargo llvm-cov --lcov` and once as `--cobertura`, which is
        // that lcov report converted by the same crate.
        let lcov = include_str!("../../tests/fixtures/lcov_demo.info");
        let expected = include_str!("../../tests/fixtures/lcov_demo.xml");
        let root = tempdir().expect("tempdir");
        fs::create_dir_all(root.path().join("src")).expect("mkdir");
        fs::write(
            root.path().join("src/lib.rs"),
            include_str!("../../tests/fixtures/lcov_demo.rs.txt"),
        )
        .expect("write");

        let got = to_cobertura(lcov, root.path()).expect("converts");
        let got = contents(&got);
        let names: Vec<&str> = got.methods.iter().map(|m| m.1.as_str()).collect();
        for want in [
            "demo::pick::<u8>",
            "demo::pick::<u16>",
            "demo::later",
            "demo::later::{closure#0}",
            "demo::total::{closure#0}",
        ] {
            assert!(names.contains(&want), "{want} in {names:?}");
        }
        assert_eq!(contents(expected), got);
    }

    #[test]
    fn the_longest_tail_that_exists_decides_the_directory() {
        // `src/lib.rs` exists under the root too, but the whole of `crates/a/src/lib.rs`
        // does, and it says where the report's directory ends.
        let root = root_with(&["src/lib.rs", "crates/a/src/lib.rs"]);
        let files = ["/build/proj/crates/a/src/lib.rs"];
        assert_eq!(base_dir(&files, root.path()), PathBuf::from("/build/proj"));
    }

    #[test]
    fn a_tie_between_directories_goes_to_the_same_one_every_time() {
        let root = root_with(&["src/a.rs", "src/b.rs"]);
        let files = ["/one/src/a.rs", "/two/src/b.rs"];
        let first = base_dir(&files, root.path());
        for _ in 0..20 {
            assert_eq!(base_dir(&files, root.path()), first);
        }
        assert_eq!(first, PathBuf::from("/one"));
    }

    #[test]
    fn files_already_under_the_source_root_are_found_by_their_whole_path() {
        let root = root_with(&["src/lib.rs"]);
        let abs = root.path().join("src/lib.rs");
        let abs = abs.to_str().expect("utf-8");
        assert_eq!(base_dir(&[abs], root.path()), root.path());
        let xml = to_cobertura(&lcov(abs), root.path()).expect("converts");
        let report = cobertura::parse(&xml).expect("parses");
        assert_eq!(report.classes[0].filename, "src/lib.rs");
    }

    #[test]
    fn nothing_found_leaves_the_names_as_they_are() {
        let root = root_with(&["unrelated.txt"]);
        let files = ["/build/demo/src/lib.rs"];
        assert_eq!(base_dir(&files, root.path()), PathBuf::new());
        let xml = to_cobertura(&lcov(files[0]), root.path()).expect("converts");
        let report = cobertura::parse(&xml).expect("parses");
        assert_eq!(report.classes[0].filename, "/build/demo/src/lib.rs");
    }

    #[test]
    fn a_relative_name_proposes_no_directory_even_when_a_tail_of_it_exists() {
        // `lib.rs` exists at the root as well as `src/lib.rs`.
        let root = root_with(&["src/lib.rs", "lib.rs"]);
        assert_eq!(base_dir(&["src/lib.rs"], root.path()), PathBuf::new());
        let xml = to_cobertura(&lcov("src/lib.rs"), root.path()).expect("converts");
        let report = cobertura::parse(&xml).expect("parses");
        assert_eq!(report.classes[0].filename, "src/lib.rs");
    }

    #[test]
    fn relative_names_are_left_alone() {
        let root = root_with(&["src/lib.rs"]);
        let xml = to_cobertura(&lcov("src/lib.rs"), root.path()).expect("converts");
        let report = cobertura::parse(&xml).expect("parses");
        assert_eq!(report.classes[0].filename, "src/lib.rs");
    }

    #[test]
    fn an_lcov_report_without_function_records_is_refused_with_the_way_to_make_one() {
        let root = root_with(&["src/lib.rs"]);
        let text = "TN:\nSF:/a/src/lib.rs\nDA:1,1\nLF:1\nLH:1\nend_of_record\n";
        let err = format!(
            "{:#}",
            to_cobertura(text, root.path()).expect_err("refused")
        );
        assert!(
            err.contains("no function records") && err.contains("cargo llvm-cov --lcov"),
            "{err}"
        );
    }

    #[test]
    fn a_record_the_converter_does_not_know_is_an_error_that_names_the_report() {
        let root = root_with(&["src/lib.rs"]);
        let text =
            "TN:\nSF:/a/src/lib.rs\nFN:1,_RNvCs2mwpCrQKxwN_4demo3add\nBOGUS:1\nend_of_record\n";
        let err = format!(
            "{:#}",
            to_cobertura(text, root.path()).expect_err("refused")
        );
        assert!(err.contains("lcov") && err.contains("BOGUS"), "{err}");
    }
}
