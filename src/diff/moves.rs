use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use super::model::{DiffLine, FileDiff, LineKind, MoveMatch};

const MIN_BLOCK_SIZE: usize = 3;
const SIMILARITY_THRESHOLD: f64 = 0.8;

/// Normalize a line by collapsing all whitespace runs into single spaces and trimming.
fn normalize(line: &str) -> String {
    line.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// Hash a normalized line using DefaultHasher.
fn hash_line(line: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

/// Compute similarity between two pre-hashed blocks.
/// Returns the fraction of matching lines (by hash) over the length of the longer block.
fn block_similarity(hashed: &[u64], other_hashed: &[u64]) -> f64 {
    let max_len = hashed.len().max(other_hashed.len());
    if max_len == 0 {
        return 0.0;
    }
    let matches = hashed
        .iter()
        .zip(other_hashed.iter())
        .filter(|(a, b)| a == b)
        .count();
    matches as f64 / max_len as f64
}

/// A contiguous block of deleted or added lines extracted from a hunk.
#[derive(Debug)]
struct Block {
    file_path: PathBuf,
    kind: LineKind,
    /// The line numbers in the block (old_line_no for Deleted, new_line_no for Added).
    start_line: usize,
    end_line: usize,
    hashed: Vec<u64>,
}

/// Extract contiguous blocks of Deleted or Added lines from all files.
fn extract_blocks(files: &[FileDiff]) -> Vec<Block> {
    let mut blocks = Vec::new();

    for file in files {
        for hunk in &file.hunks {
            let mut current_kind: Option<LineKind> = None;
            let mut current_lines: Vec<&DiffLine> = Vec::new();

            for line in &hunk.lines {
                if line.kind == LineKind::Deleted || line.kind == LineKind::Added {
                    if current_kind == Some(line.kind) {
                        current_lines.push(line);
                    } else {
                        // Flush the previous block if it meets the minimum size.
                        if let Some(kind) = current_kind
                            && current_lines.len() >= MIN_BLOCK_SIZE
                        {
                            blocks.push(build_block(
                                &file.path,
                                kind,
                                &current_lines,
                            ));
                        }
                        current_kind = Some(line.kind);
                        current_lines = vec![line];
                    }
                } else {
                    // Non-deleted/added line breaks the block.
                    if let Some(kind) = current_kind
                        && current_lines.len() >= MIN_BLOCK_SIZE
                    {
                        blocks.push(build_block(&file.path, kind, &current_lines));
                    }
                    current_kind = None;
                    current_lines.clear();
                }
            }

            // Flush any remaining block at end of hunk.
            if let Some(kind) = current_kind
                && current_lines.len() >= MIN_BLOCK_SIZE
            {
                blocks.push(build_block(&file.path, kind, &current_lines));
            }
        }
    }

    blocks
}

/// Build a Block from a slice of DiffLines.
fn build_block(file_path: &Path, kind: LineKind, lines: &[&DiffLine]) -> Block {
    let text_fn = |line: &DiffLine| -> String {
        let raw = match kind {
            LineKind::Deleted => line.old_str(),
            LineKind::Added => line.new_str(),
            _ => "",
        };
        normalize(raw)
    };

    let line_no_fn = |line: &DiffLine| -> usize {
        match kind {
            LineKind::Deleted => line.old_line_no.unwrap_or(0),
            LineKind::Added => line.new_line_no.unwrap_or(0),
            _ => 0,
        }
    };

    let hashed: Vec<u64> = lines.iter().map(|l| hash_line(&text_fn(l))).collect();
    let start_line = line_no_fn(lines[0]);
    let end_line = line_no_fn(lines[lines.len() - 1]);

    Block {
        file_path: file_path.to_path_buf(),
        kind,
        start_line,
        end_line,
        hashed,
    }
}

/// Detect moved blocks across all files and populate `move_matches` on each `FileDiff`.
///
/// A "move" is a deleted block that reappears as an added block (possibly in a different file)
/// with similarity >= 80%.
pub fn detect_moves(files: &mut [FileDiff]) {
    let blocks = extract_blocks(files);

    let deleted_blocks: Vec<&Block> = blocks.iter().filter(|b| b.kind == LineKind::Deleted).collect();
    let added_blocks: Vec<&Block> = blocks.iter().filter(|b| b.kind == LineKind::Added).collect();

    let mut matches: Vec<MoveMatch> = Vec::new();
    let mut used_added: Vec<bool> = vec![false; added_blocks.len()];

    for del in &deleted_blocks {
        let mut best_idx: Option<usize> = None;
        let mut best_sim: f64 = 0.0;

        for (i, add) in added_blocks.iter().enumerate() {
            if used_added[i] {
                continue;
            }
            let sim = block_similarity(&del.hashed, &add.hashed);
            if sim >= SIMILARITY_THRESHOLD && sim > best_sim {
                best_sim = sim;
                best_idx = Some(i);
            }
        }

        if let Some(idx) = best_idx {
            used_added[idx] = true;
            let add = added_blocks[idx];
            matches.push(MoveMatch {
                source_file: del.file_path.clone(),
                source_start: del.start_line,
                source_end: del.end_line,
                dest_file: add.file_path.clone(),
                dest_start: add.start_line,
                dest_end: add.end_line,
                similarity: best_sim,
            });
        }
    }

    // Distribute matches back to the relevant FileDiff entries.
    for file in files.iter_mut() {
        for m in &matches {
            if m.source_file == file.path || m.dest_file == file.path {
                file.move_matches.push(m.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::model::*;
    use std::path::PathBuf;

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("  let   x = 1;  "), "let x = 1;");
    }

    #[test]
    fn test_identical_blocks_match() {
        let lines = vec![
            "fn foo() {",
            "    let x = 1;",
            "    let y = 2;",
            "    x + y",
        ];
        let normalized: Vec<String> = lines.iter().map(|l| normalize(l)).collect();
        let hashed: Vec<u64> = normalized.iter().map(|l| hash_line(l)).collect();
        let sim = block_similarity(&hashed, &hashed);
        assert!((sim - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_detect_move_within_file() {
        let moved_lines = vec![
            "fn moved_function() {",
            "    let a = 10;",
            "    let b = 20;",
            "    a + b",
        ];

        // Build a FileDiff with one hunk that has 4 deleted lines then 4 added lines
        // (representing a function that was moved within the file).
        let mut deleted_diff_lines: Vec<DiffLine> = moved_lines
            .iter()
            .enumerate()
            .map(|(i, text)| DiffLine {
                kind: LineKind::Deleted,
                old_line_no: Some(10 + i),
                new_line_no: None,
                old_text: Some(text.to_string()),
                new_text: None,
                tokens: vec![],
            })
            .collect();

        // Add a context line to break the blocks apart.
        deleted_diff_lines.push(DiffLine {
            kind: LineKind::Context,
            old_line_no: Some(14),
            new_line_no: Some(14),
            old_text: Some("// separator".to_string()),
            new_text: Some("// separator".to_string()),
            tokens: vec![],
        });

        let added_diff_lines: Vec<DiffLine> = moved_lines
            .iter()
            .enumerate()
            .map(|(i, text)| DiffLine {
                kind: LineKind::Added,
                old_line_no: None,
                new_line_no: Some(50 + i),
                old_text: None,
                new_text: Some(text.to_string()),
                tokens: vec![],
            })
            .collect();

        let mut all_lines = deleted_diff_lines;
        all_lines.extend(added_diff_lines);

        let mut files = vec![FileDiff {
            path: PathBuf::from("src/main.rs"),
            status: FileStatus::Modified,
            hunks: vec![Hunk {
                old_start: 10,
                new_start: 10,
                old_lines: 5,
                new_lines: 5,
                lines: all_lines,
            }],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }];

        detect_moves(&mut files);

        assert_eq!(files[0].move_matches.len(), 1);
        let m = &files[0].move_matches[0];
        assert_eq!(m.source_file, PathBuf::from("src/main.rs"));
        assert_eq!(m.dest_file, PathBuf::from("src/main.rs"));
        assert_eq!(m.source_start, 10);
        assert_eq!(m.source_end, 13);
        assert_eq!(m.dest_start, 50);
        assert_eq!(m.dest_end, 53);
        assert!((m.similarity - 1.0).abs() < f64::EPSILON);
    }
}
