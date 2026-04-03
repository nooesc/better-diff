//! Parse unified diff / patch format into `FileDiff` structs.
//!
//! Handles output from `git diff`, `diff -u`, and similar tools.

use std::path::PathBuf;

use crate::diff::model::{DiffLine, FileDiff, FileStatus, Hunk, LineKind};
use crate::diff::tokens::compute_token_changes;

/// Parse a unified diff string into a list of `FileDiff` entries.
pub fn parse_patch(input: &str) -> Vec<FileDiff> {
    let lines: Vec<&str> = input.lines().collect();
    let mut files: Vec<FileDiff> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Look for "diff --git" or "--- " header
        if lines[i].starts_with("diff --git ") {
            let (file_diff, next) = parse_git_diff_entry(&lines, i);
            if let Some(fd) = file_diff {
                files.push(fd);
            }
            i = next;
        } else if lines[i].starts_with("--- ") && i + 1 < lines.len() && lines[i + 1].starts_with("+++ ") {
            // Plain unified diff (no "diff --git" header)
            let (file_diff, next) = parse_plain_diff_entry(&lines, i);
            if let Some(fd) = file_diff {
                files.push(fd);
            }
            i = next;
        } else {
            i += 1;
        }
    }

    files
}

/// Parse a single file entry starting at a "diff --git" line.
fn parse_git_diff_entry(lines: &[&str], start: usize) -> (Option<FileDiff>, usize) {
    let header = lines[start];
    let mut i = start + 1;

    // Extract paths from "diff --git a/path b/path"
    let (old_path, new_path) = parse_git_diff_header(header);

    let mut status = FileStatus::Modified;
    let mut old_path_resolved = old_path.clone();
    let mut new_path_resolved = new_path.clone();

    // Skip extended headers (index, old mode, new mode, similarity, rename, etc.)
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("new file mode") {
            status = FileStatus::Added;
            i += 1;
        } else if line.starts_with("deleted file mode") {
            status = FileStatus::Deleted;
            i += 1;
        } else if line.starts_with("rename from ") {
            old_path_resolved = Some(PathBuf::from(line.strip_prefix("rename from ").unwrap()));
            status = FileStatus::Renamed;
            i += 1;
        } else if line.starts_with("rename to ") {
            new_path_resolved = Some(PathBuf::from(line.strip_prefix("rename to ").unwrap()));
            i += 1;
        } else if line.starts_with("similarity index")
            || line.starts_with("dissimilarity index")
            || line.starts_with("index ")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
            || line.starts_with("copy from")
            || line.starts_with("copy to")
        {
            i += 1;
        } else if line.starts_with("Binary files") {
            // Skip binary files
            return (None, i + 1);
        } else {
            break;
        }
    }

    // Now expect "--- " and "+++ " lines
    if i + 1 < lines.len() && lines[i].starts_with("--- ") && lines[i + 1].starts_with("+++ ") {
        let old_file = strip_diff_prefix(lines[i].strip_prefix("--- ").unwrap());
        let new_file = strip_diff_prefix(lines[i + 1].strip_prefix("+++ ").unwrap());
        if old_path_resolved.is_none() {
            old_path_resolved = old_file;
        }
        if new_path_resolved.is_none() {
            new_path_resolved = new_file;
        }
        i += 2;
    }

    let path = new_path_resolved
        .clone()
        .or(old_path_resolved.clone())
        .unwrap_or_else(|| PathBuf::from("unknown"));

    let old_path_field = if status == FileStatus::Renamed {
        old_path_resolved
    } else {
        None
    };

    // Parse hunks
    let (hunks, next_i) = parse_hunks(lines, i);

    // Reconstruct old/new content from hunks
    let (old_content, new_content) = reconstruct_content(&hunks);

    // Run token-level diffing on Modified lines
    let hunks = add_token_changes(hunks);

    let file_diff = FileDiff {
        path,
        old_path: old_path_field,
        status,
        hunks,
        old_content,
        new_content,
        fold_regions: vec![],
        move_matches: vec![],
    };

    (Some(file_diff), next_i)
}

/// Parse a plain unified diff entry starting at "--- " line.
fn parse_plain_diff_entry(lines: &[&str], start: usize) -> (Option<FileDiff>, usize) {
    let old_file = strip_diff_prefix(lines[start].strip_prefix("--- ").unwrap());
    let new_file = strip_diff_prefix(lines[start + 1].strip_prefix("+++ ").unwrap());

    let path = new_file
        .clone()
        .or(old_file.clone())
        .unwrap_or_else(|| PathBuf::from("unknown"));

    let status = if old_file.as_ref().map_or(false, |p| p.to_str() == Some("/dev/null")) {
        FileStatus::Added
    } else if new_file.as_ref().map_or(false, |p| p.to_str() == Some("/dev/null")) {
        FileStatus::Deleted
    } else {
        FileStatus::Modified
    };

    let (hunks, next_i) = parse_hunks(lines, start + 2);
    let (old_content, new_content) = reconstruct_content(&hunks);
    let hunks = add_token_changes(hunks);

    let file_diff = FileDiff {
        path,
        old_path: None,
        status,
        hunks,
        old_content,
        new_content,
        fold_regions: vec![],
        move_matches: vec![],
    };

    (Some(file_diff), next_i)
}

/// Extract old and new paths from "diff --git a/foo b/bar".
fn parse_git_diff_header(header: &str) -> (Option<PathBuf>, Option<PathBuf>) {
    let rest = header.strip_prefix("diff --git ").unwrap_or("");
    // Try splitting on " b/" — handles the common case
    if let Some(idx) = rest.find(" b/") {
        let old_part = &rest[..idx];
        let new_part = &rest[idx + 1..];
        let old = old_part.strip_prefix("a/").unwrap_or(old_part);
        let new = new_part.strip_prefix("b/").unwrap_or(new_part);
        (Some(PathBuf::from(old)), Some(PathBuf::from(new)))
    } else {
        (None, None)
    }
}

/// Strip "a/" or "b/" prefix and handle /dev/null.
fn strip_diff_prefix(path: &str) -> Option<PathBuf> {
    let trimmed = path.trim();
    if trimmed == "/dev/null" {
        return None;
    }
    let stripped = trimmed
        .strip_prefix("a/")
        .or_else(|| trimmed.strip_prefix("b/"))
        .unwrap_or(trimmed);
    Some(PathBuf::from(stripped))
}

/// Parse all hunks starting from position `i` until the next file or end of input.
fn parse_hunks(lines: &[&str], start: usize) -> (Vec<Hunk>, usize) {
    let mut hunks = Vec::new();
    let mut i = start;

    while i < lines.len() {
        if lines[i].starts_with("@@ ") {
            let (hunk, next) = parse_single_hunk(lines, i);
            hunks.push(hunk);
            i = next;
        } else if lines[i].starts_with("diff --git ")
            || (lines[i].starts_with("--- ") && i + 1 < lines.len() && lines[i + 1].starts_with("+++ "))
        {
            break;
        } else {
            i += 1;
        }
    }

    (hunks, i)
}

/// Parse a single hunk starting at an "@@ ... @@" line.
fn parse_single_hunk(lines: &[&str], start: usize) -> (Hunk, usize) {
    let header = lines[start];
    let (old_start, old_lines, new_start, new_lines) = parse_hunk_header(header);

    let mut diff_lines = Vec::new();
    let mut old_line = old_start;
    let mut new_line = new_start;
    let mut i = start + 1;

    while i < lines.len() {
        let line = lines[i];

        if line.starts_with("@@ ")
            || line.starts_with("diff --git ")
            || (line.starts_with("--- ") && i + 1 < lines.len() && lines[i + 1].starts_with("+++ "))
        {
            break;
        }

        if line.starts_with(' ') || (line.is_empty() && !line.starts_with('\\')) {
            let text = if line.is_empty() { "" } else { &line[1..] };
            diff_lines.push(DiffLine {
                kind: LineKind::Context,
                old_line_no: Some(old_line),
                new_line_no: Some(new_line),
                old_text: Some(text.to_string()),
                new_text: Some(text.to_string()),
                tokens: vec![],
            });
            old_line += 1;
            new_line += 1;
        } else if let Some(text) = line.strip_prefix('+') {
            diff_lines.push(DiffLine {
                kind: LineKind::Added,
                old_line_no: None,
                new_line_no: Some(new_line),
                old_text: None,
                new_text: Some(text.to_string()),
                tokens: vec![],
            });
            new_line += 1;
        } else if let Some(text) = line.strip_prefix('-') {
            diff_lines.push(DiffLine {
                kind: LineKind::Deleted,
                old_line_no: Some(old_line),
                new_line_no: None,
                old_text: Some(text.to_string()),
                new_text: None,
                tokens: vec![],
            });
            old_line += 1;
        } else if line.starts_with('\\') {
            // "\ No newline at end of file" — skip
        } else {
            // Unknown line — treat as context
            break;
        }

        i += 1;
    }

    let hunk = Hunk {
        old_start,
        new_start,
        old_lines,
        new_lines,
        lines: diff_lines,
    };

    (hunk, i)
}

/// Parse "@@ -old_start,old_lines +new_start,new_lines @@" header.
fn parse_hunk_header(header: &str) -> (usize, usize, usize, usize) {
    // Strip leading "@@ " and trailing " @@..."
    let inner = header
        .strip_prefix("@@ ")
        .and_then(|s| {
            if let Some(end) = s.find(" @@") {
                Some(&s[..end])
            } else {
                None
            }
        })
        .unwrap_or("");

    let parts: Vec<&str> = inner.split_whitespace().collect();
    let (old_start, old_lines) = if let Some(old) = parts.first() {
        parse_range(old.strip_prefix('-').unwrap_or(old))
    } else {
        (1, 0)
    };
    let (new_start, new_lines) = if let Some(new) = parts.get(1) {
        parse_range(new.strip_prefix('+').unwrap_or(new))
    } else {
        (1, 0)
    };

    (old_start, old_lines, new_start, new_lines)
}

/// Parse "start,count" or "start" into (start, count).
fn parse_range(s: &str) -> (usize, usize) {
    if let Some((start, count)) = s.split_once(',') {
        (
            start.parse().unwrap_or(1),
            count.parse().unwrap_or(0),
        )
    } else {
        (s.parse().unwrap_or(1), 1)
    }
}

/// Reconstruct old and new file content from parsed hunks.
fn reconstruct_content(hunks: &[Hunk]) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();

    for hunk in hunks {
        for line in &hunk.lines {
            match line.kind {
                LineKind::Context => {
                    if let Some(ref text) = line.old_text {
                        old.push_str(text);
                        old.push('\n');
                    }
                    if let Some(ref text) = line.new_text {
                        new.push_str(text);
                        new.push('\n');
                    }
                }
                LineKind::Added => {
                    if let Some(ref text) = line.new_text {
                        new.push_str(text);
                        new.push('\n');
                    }
                }
                LineKind::Deleted => {
                    if let Some(ref text) = line.old_text {
                        old.push_str(text);
                        old.push('\n');
                    }
                }
                LineKind::Modified => {
                    if let Some(ref text) = line.old_text {
                        old.push_str(text);
                        old.push('\n');
                    }
                    if let Some(ref text) = line.new_text {
                        new.push_str(text);
                        new.push('\n');
                    }
                }
            }
        }
    }

    (old, new)
}

/// Convert adjacent Deleted+Added pairs into Modified lines with token changes.
fn add_token_changes(hunks: Vec<Hunk>) -> Vec<Hunk> {
    hunks
        .into_iter()
        .map(|mut hunk| {
            let mut new_lines = Vec::new();
            let mut i = 0;
            while i < hunk.lines.len() {
                if hunk.lines[i].kind == LineKind::Deleted
                    && i + 1 < hunk.lines.len()
                    && hunk.lines[i + 1].kind == LineKind::Added
                {
                    let old_text = hunk.lines[i].old_str().to_string();
                    let new_text = hunk.lines[i + 1].new_str().to_string();
                    let (old_tokens, new_tokens) = compute_token_changes(&old_text, &new_text);

                    // Emit old-side Modified line
                    new_lines.push(DiffLine {
                        kind: LineKind::Modified,
                        old_line_no: hunk.lines[i].old_line_no,
                        new_line_no: None,
                        old_text: Some(old_text),
                        new_text: None,
                        tokens: old_tokens,
                    });
                    // Emit new-side Modified line
                    new_lines.push(DiffLine {
                        kind: LineKind::Modified,
                        old_line_no: None,
                        new_line_no: hunk.lines[i + 1].new_line_no,
                        old_text: None,
                        new_text: Some(new_text),
                        tokens: new_tokens,
                    });
                    i += 2;
                } else {
                    new_lines.push(hunk.lines[i].clone());
                    i += 1;
                }
            }
            hunk.lines = new_lines;
            hunk
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hunk_header() {
        assert_eq!(parse_hunk_header("@@ -1,3 +1,4 @@"), (1, 3, 1, 4));
        assert_eq!(parse_hunk_header("@@ -10,0 +11,2 @@ fn main()"), (10, 0, 11, 2));
        assert_eq!(parse_hunk_header("@@ -1 +1 @@"), (1, 1, 1, 1));
    }

    #[test]
    fn test_parse_simple_patch() {
        let patch = "\
diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!(\"hello\");
+    println!(\"world\");
 }
";
        let files = parse_patch(patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("src/main.rs"));
        assert_eq!(files[0].status, FileStatus::Modified);
        assert_eq!(files[0].hunks.len(), 1);
        // Should have been converted to Modified pair
        assert!(files[0].hunks[0].lines.iter().any(|l| l.kind == LineKind::Modified));
    }

    #[test]
    fn test_parse_added_file() {
        let patch = "\
diff --git a/new_file.txt b/new_file.txt
new file mode 100644
--- /dev/null
+++ b/new_file.txt
@@ -0,0 +1,2 @@
+line 1
+line 2
";
        let files = parse_patch(patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Added);
    }

    #[test]
    fn test_parse_deleted_file() {
        let patch = "\
diff --git a/old_file.txt b/old_file.txt
deleted file mode 100644
--- a/old_file.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-line 1
-line 2
";
        let files = parse_patch(patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Deleted);
    }

    #[test]
    fn test_parse_renamed_file() {
        let patch = "\
diff --git a/old_name.rs b/new_name.rs
similarity index 95%
rename from old_name.rs
rename to new_name.rs
--- a/old_name.rs
+++ b/new_name.rs
@@ -1,3 +1,3 @@
 fn main() {
-    old();
+    new();
 }
";
        let files = parse_patch(patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(files[0].path, PathBuf::from("new_name.rs"));
        assert_eq!(files[0].old_path, Some(PathBuf::from("old_name.rs")));
    }

    #[test]
    fn test_parse_multiple_files() {
        let patch = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,1 +1,1 @@
-old a
+new a
diff --git a/b.rs b/b.rs
--- a/b.rs
+++ b/b.rs
@@ -1,1 +1,1 @@
-old b
+new b
";
        let files = parse_patch(patch);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, PathBuf::from("a.rs"));
        assert_eq!(files[1].path, PathBuf::from("b.rs"));
    }

    #[test]
    fn test_reconstruct_content() {
        let hunks = vec![Hunk {
            old_start: 1,
            new_start: 1,
            old_lines: 3,
            new_lines: 3,
            lines: vec![
                DiffLine {
                    kind: LineKind::Context,
                    old_line_no: Some(1),
                    new_line_no: Some(1),
                    old_text: Some("fn main() {".to_string()),
                    new_text: Some("fn main() {".to_string()),
                    tokens: vec![],
                },
                DiffLine {
                    kind: LineKind::Deleted,
                    old_line_no: Some(2),
                    new_line_no: None,
                    old_text: Some("    old();".to_string()),
                    new_text: None,
                    tokens: vec![],
                },
                DiffLine {
                    kind: LineKind::Added,
                    old_line_no: None,
                    new_line_no: Some(2),
                    old_text: None,
                    new_text: Some("    new();".to_string()),
                    tokens: vec![],
                },
                DiffLine {
                    kind: LineKind::Context,
                    old_line_no: Some(3),
                    new_line_no: Some(3),
                    old_text: Some("}".to_string()),
                    new_text: Some("}".to_string()),
                    tokens: vec![],
                },
            ],
        }];

        let (old, new) = reconstruct_content(&hunks);
        assert_eq!(old, "fn main() {\n    old();\n}\n");
        assert_eq!(new, "fn main() {\n    new();\n}\n");
    }
}
