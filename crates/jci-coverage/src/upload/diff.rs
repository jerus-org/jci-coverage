//! Strips a `git diff --unified=0` patch down to the shape OtterWise expects
//! for diff coverage: hunk markers and changed lines only, no file-header or
//! index noise. Mirrors the reference `getOtterWise/bash-uploader` script's
//! strip rules.

/// Whether a line from a unified diff should be kept.
fn keep_line(line: &str) -> bool {
    !(line.starts_with("diff --git")
        || line.starts_with("new file mode")
        || line.starts_with("deleted file mode")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("Binary files ")
        || line.starts_with("similarity index ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
        || line.starts_with("copy from ")
        || line.starts_with("copy to "))
}

/// Strip a raw unified diff down to hunk markers and changed lines.
pub fn strip(raw: &str) -> String {
    raw.lines()
        .filter(|line| keep_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "diff --git a/src/lib.rs b/src/lib.rs\n\
index 1234567..89abcde 100644\n\
--- a/src/lib.rs\n\
+++ b/src/lib.rs\n\
@@ -1,0 +2 @@\n\
+// a new comment\n";

    #[test]
    fn strips_file_header_and_index_lines() {
        let stripped = strip(RAW);
        assert!(!stripped.contains("diff --git"));
        assert!(!stripped.contains("index "));
        assert!(!stripped.contains("--- a/"));
        assert!(!stripped.contains("+++ b/"));
    }

    #[test]
    fn keeps_hunk_markers_and_changed_lines() {
        let stripped = strip(RAW);
        assert!(stripped.contains("@@ -1,0 +2 @@"));
        assert!(stripped.contains("+// a new comment"));
    }

    #[test]
    fn keeps_new_file_and_deleted_file_diffs_minus_their_mode_lines() {
        let raw = "diff --git a/new.rs b/new.rs\n\
new file mode 100644\n\
index 0000000..1234567\n\
--- /dev/null\n\
+++ b/new.rs\n\
@@ -0,0 +1 @@\n\
+fn added() {}\n";
        let stripped = strip(raw);
        assert!(!stripped.contains("new file mode"));
        assert!(stripped.contains("+fn added() {}"));

        let raw = raw.replace("new file mode", "deleted file mode");
        let stripped = strip(&raw);
        assert!(!stripped.contains("deleted file mode"));
    }

    #[test]
    fn empty_diff_strips_to_empty_string() {
        assert_eq!(strip(""), "");
    }

    #[test]
    fn strips_binary_file_marker() {
        let raw = "diff --git a/img.png b/img.png\n\
index 1234567..89abcde 100644\n\
Binary files a/img.png and b/img.png differ\n";
        assert_eq!(strip(raw), "");
    }

    #[test]
    fn strips_rename_and_copy_metadata_but_keeps_the_hunk() {
        let raw = "diff --git a/old.rs b/new.rs\n\
similarity index 100%\n\
rename from old.rs\n\
rename to new.rs\n";
        assert_eq!(strip(raw), "");

        let raw = "diff --git a/a.rs b/b.rs\n\
similarity index 90%\n\
copy from a.rs\n\
copy to b.rs\n\
@@ -1 +1 @@\n\
-old\n\
+new\n";
        let stripped = strip(raw);
        assert!(!stripped.contains("copy from"));
        assert!(!stripped.contains("copy to"));
        assert!(stripped.contains("-old"));
        assert!(stripped.contains("+new"));
    }
}
