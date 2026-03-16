use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use super::model::{DiffLine, FileDiff, LineKind, MoveMatch};

const MIN_BLOCK_SIZE: usize = 3;
const SIMILARITY_THRESHOLD: f64 = 0.8;
const LENGTH_BIAS: f64 = 0.25;
const FILE_DISTANCE_BONUS: f64 = 0.15;

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

fn can_reach_similarity_threshold(a: usize, b: usize, threshold: f64) -> bool {
    let min_len = a.min(b) as f64;
    let max_len = a.max(b) as f64;

    if max_len == 0.0 {
        return false;
    }

    (min_len / max_len) >= threshold
}

/// A contiguous block of deleted or added lines extracted from a hunk.
#[derive(Debug)]
struct Block {
    file_path: PathBuf,
    kind: LineKind,
    /// The line numbers in the block (old_line_no for Deleted, new_line_no for Added).
    start_line: usize,
    end_line: usize,
    line_count: usize,
    hashed: Vec<u64>,
}

/// Extract contiguous blocks of Deleted or Added lines from all files.
fn extract_blocks(files: &[FileDiff]) -> Vec<Block> {
    let mut blocks = Vec::new();

    for file in files {
        let block_file_path = |kind: LineKind| -> &Path {
            match kind {
                LineKind::Deleted => file.old_path.as_deref().unwrap_or(&file.path),
                LineKind::Added => &file.path,
                _ => &file.path,
            }
        };

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
                        blocks.push(build_block(block_file_path(kind), kind, &current_lines));
                        }
                        current_kind = Some(line.kind);
                        current_lines = vec![line];
                    }
                } else {
                    // Non-deleted/added line breaks the block.
                    if let Some(kind) = current_kind
                        && current_lines.len() >= MIN_BLOCK_SIZE
                    {
                        blocks.push(build_block(block_file_path(kind), kind, &current_lines));
                    }
                    current_kind = None;
                    current_lines.clear();
                }
            }

            // Flush any remaining block at end of hunk.
            if let Some(kind) = current_kind
                && current_lines.len() >= MIN_BLOCK_SIZE
            {
                blocks.push(build_block(block_file_path(kind), kind, &current_lines));
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
    let line_count = lines.len();

    Block {
        file_path: file_path.to_path_buf(),
        kind,
        start_line,
        end_line,
        line_count,
        hashed,
    }
}

fn line_overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    let a_start = a_start.min(a_end);
    let a_end = a_end.max(a_start);
    let b_start = b_start.min(b_end);
    let b_end = b_end.max(b_start);

    !(a_end < b_start || b_end < a_start)
}

fn block_score(similarity: f64, del: &Block, add: &Block) -> f64 {
    let length_penalty = if del.line_count == 0 || add.line_count == 0 {
        0.0
    } else {
        del.line_count.min(add.line_count) as f64 / del.line_count.max(add.line_count) as f64
    };

    let same_file_bonus = if del.file_path == add.file_path {
        let same_file_distance = (del.start_line as i64 - add.start_line as i64).abs() as f64;
        FILE_DISTANCE_BONUS * (1.0 / (1.0 + same_file_distance / 20.0))
    } else {
        0.0
    };

    similarity * (1.0 - LENGTH_BIAS) + length_penalty * LENGTH_BIAS + same_file_bonus
}

/// Detect moved blocks across all files and populate `move_matches` on each `FileDiff`.
///
/// A "move" is a deleted block that reappears as an added block (possibly in a different file)
/// with similarity >= 80%.
pub fn detect_moves(files: &mut [FileDiff]) {
    let blocks = extract_blocks(files);

    let deleted_blocks: Vec<(usize, &Block)> = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.kind == LineKind::Deleted)
        .collect();
    let added_blocks: Vec<(usize, &Block)> = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.kind == LineKind::Added)
        .collect();

    let mut matches: Vec<MoveMatch> = Vec::new();
    let mut used_added = vec![false; blocks.len()];
    let mut used_deleted = vec![false; blocks.len()];
    let mut used_source_lines: Vec<(PathBuf, usize, usize)> = Vec::new();
    let mut used_dest_lines: Vec<(PathBuf, usize, usize)> = Vec::new();

    let mut candidates = build_move_candidates(&deleted_blocks, &added_blocks);

    candidates.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| blocks[a.2].line_count.cmp(&blocks[b.2].line_count).reverse())
            .then_with(|| blocks[a.3].line_count.cmp(&blocks[b.3].line_count).reverse())
            .then_with(|| blocks[a.2].file_path.cmp(&blocks[b.2].file_path))
            .then_with(|| blocks[a.2].start_line.cmp(&blocks[b.2].start_line))
            .then_with(|| blocks[a.3].file_path.cmp(&blocks[b.3].file_path))
            .then_with(|| blocks[a.3].start_line.cmp(&blocks[b.3].start_line))
    });

    for (_score, sim, del_idx, add_idx) in candidates {
        if used_deleted[del_idx] || used_added[add_idx] {
            continue;
        }

        let del = &blocks[del_idx];
        let add = &blocks[add_idx];

        if used_source_lines
            .iter()
            .any(|(file_path, s, e)| {
                file_path == &del.file_path && line_overlap(del.start_line, del.end_line, *s, *e)
            })
            || used_dest_lines
                .iter()
                .any(|(file_path, s, e)| {
                    file_path == &add.file_path && line_overlap(add.start_line, add.end_line, *s, *e)
                })
        {
            continue;
        }

        used_deleted[del_idx] = true;
        used_added[add_idx] = true;
        used_source_lines.push((del.file_path.clone(), del.start_line, del.end_line));
        used_dest_lines.push((add.file_path.clone(), add.start_line, add.end_line));

        matches.push(MoveMatch {
            source_file: del.file_path.clone(),
            source_start: del.start_line,
            source_end: del.end_line,
            dest_file: add.file_path.clone(),
            dest_start: add.start_line,
            dest_end: add.end_line,
            similarity: sim,
        });
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

fn build_move_candidates(
    deleted_blocks: &[(usize, &Block)],
    added_blocks: &[(usize, &Block)],
) -> Vec<(f64, f64, usize, usize)> {
    // Consider all possible candidates and rank by similarity, then stable tie-breakers.
    let mut candidates: Vec<(f64, f64, usize, usize)> = Vec::new();
    let mut added_by_length: Vec<Vec<(usize, &Block)>> = Vec::new();
    for &(idx, b) in added_blocks {
        if added_by_length.len() <= b.line_count {
            added_by_length.resize_with(b.line_count + 1, Vec::new);
        }
        added_by_length[b.line_count].push((idx, b));
    }

    for &(del_idx, del) in deleted_blocks {
        let min_candidate_len = ((del.line_count * 4) + 4) / 5;
        let max_candidate_len = (del.line_count * 5) / 4;

        if added_by_length.is_empty() {
            continue;
        }

        let max_lookup_len = added_by_length.len() - 1;
        let min_lookup_len = min_candidate_len.min(max_lookup_len);
        let max_lookup_len = max_candidate_len.min(max_lookup_len);

        for add_len in min_lookup_len..=max_lookup_len {
            for &(add_idx, add) in &added_by_length[add_len] {
                if !can_reach_similarity_threshold(del.line_count, add.line_count, SIMILARITY_THRESHOLD) {
                    continue;
                }

                let sim = block_similarity(&del.hashed, &add.hashed);
                if sim >= SIMILARITY_THRESHOLD {
                    let score = block_score(sim, del, add);
                    candidates.push((score, sim, del_idx, add_idx));
                }
            }
        }
    }
    candidates
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
            old_path: None,
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
        assert!(
            (m.similarity - 1.0).abs() < f64::EPSILON,
            "similarity was {}",
            m.similarity
        );
    }

    #[test]
    fn test_detect_move_within_length_ratio_window() {
        let deleted_lines = vec![
            "fn moved_function() {",
            "    let a = 10;",
            "    let b = 20;",
            "    let c = 30;",
        ];
        let added_lines = vec![
            "fn moved_function() {",
            "    let a = 10;",
            "    let b = 20;",
            "    let c = 30;",
            "    let d = 40;",
        ];

        let mut deleted_diff_lines: Vec<DiffLine> = deleted_lines
            .iter()
            .enumerate()
            .map(|(i, text)| DiffLine {
                kind: LineKind::Deleted,
                old_line_no: Some(10 + i),
                new_line_no: None,
                old_text: Some((*text).to_string()),
                new_text: None,
                tokens: vec![],
            })
            .collect();

        // Break the blocks.
        deleted_diff_lines.push(DiffLine {
            kind: LineKind::Context,
            old_line_no: Some(14),
            new_line_no: Some(14),
            old_text: Some("// separator".to_string()),
            new_text: Some("// separator".to_string()),
            tokens: vec![],
        });

        let mut added_diff_lines: Vec<DiffLine> = added_lines
            .iter()
            .enumerate()
            .map(|(i, text)| DiffLine {
                kind: LineKind::Added,
                old_line_no: None,
                new_line_no: Some(50 + i),
                old_text: None,
                new_text: Some((*text).to_string()),
                tokens: vec![],
            })
            .collect();

        // Keep the added block similar enough to stay in threshold.
        let mut all_lines = deleted_diff_lines;
        all_lines.append(&mut added_diff_lines);

        let mut files = vec![FileDiff {
            path: PathBuf::from("src/main.rs"),
            old_path: None,
            status: FileStatus::Modified,
            hunks: vec![Hunk {
                old_start: 10,
                new_start: 10,
                old_lines: 5,
                new_lines: 6,
                lines: all_lines,
            }],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }];

        detect_moves(&mut files);

        assert_eq!(files[0].move_matches.len(), 1);
        assert_eq!(files[0].move_matches[0].source_file, PathBuf::from("src/main.rs"));
        assert_eq!(files[0].move_matches[0].dest_file, PathBuf::from("src/main.rs"));
    }

    #[test]
    fn test_detect_moves_do_not_share_overlap_across_files() {
        let moved_lines = ["fn moved_block() {", "    println!(\"first\");", "}"];

        fn moved_file(path: &str, start: usize, moved_lines: &[&str; 3]) -> FileDiff {
            let mut lines: Vec<DiffLine> = moved_lines
                .iter()
                .enumerate()
                .map(|(i, text)| DiffLine {
                    kind: LineKind::Deleted,
                    old_line_no: Some(start + i),
                    new_line_no: None,
                    old_text: Some((*text).to_string()),
                    new_text: None,
                    tokens: vec![],
                })
                .collect();

            lines.push(DiffLine {
                kind: LineKind::Context,
                old_line_no: Some(start + 3),
                new_line_no: Some(start + 3),
                old_text: Some("// separator".to_string()),
                new_text: Some("// separator".to_string()),
                tokens: vec![],
            });

            lines.extend(
                moved_lines
                    .iter()
                    .enumerate()
                    .map(|(i, text)| DiffLine {
                        kind: LineKind::Added,
                        old_line_no: None,
                        new_line_no: Some(start + i),
                        old_text: None,
                        new_text: Some((*text).to_string()),
                        tokens: vec![],
                    }),
            );

            FileDiff {
                path: PathBuf::from(path),
                old_path: None,
                status: FileStatus::Modified,
                hunks: vec![Hunk {
                    old_start: start,
                    new_start: start,
                    old_lines: 4,
                    new_lines: 4,
                    lines,
                }],
                old_content: String::new(),
                new_content: String::new(),
                fold_regions: vec![],
                move_matches: vec![],
            }
        }

        let mut files = vec![moved_file("src/a.rs", 10, &moved_lines), moved_file("src/b.rs", 10, &moved_lines)];

        detect_moves(&mut files);

        assert_eq!(
            files[0].move_matches.len(),
            1,
            "first file should keep its move match"
        );
        assert_eq!(
            files[1].move_matches.len(),
            1,
            "second file should keep its move match"
        );

        assert_eq!(files[0].move_matches[0].source_file, PathBuf::from("src/a.rs"));
        assert_eq!(files[1].move_matches[0].source_file, PathBuf::from("src/b.rs"));
    }

    fn build_synthetic_file_with_block_sizes(
        path: &str,
        deleted_sizes: &[usize],
        added_sizes: &[usize],
    ) -> FileDiff {
        let mut lines = Vec::new();
        let mut old_line = 1usize;
        let mut new_line = 1usize;

        let push_context = |lines: &mut Vec<DiffLine>, old_line: &mut usize, new_line: &mut usize| {
            lines.push(DiffLine {
                kind: LineKind::Context,
                old_line_no: Some(*old_line),
                new_line_no: Some(*new_line),
                old_text: Some("// gap".to_string()),
                new_text: Some("// gap".to_string()),
                tokens: vec![],
            });
            *old_line += 1;
            *new_line += 1;
        };

        for (idx, &size) in deleted_sizes.iter().enumerate() {
            for i in 0..size {
                lines.push(DiffLine {
                    kind: LineKind::Deleted,
                    old_line_no: Some(old_line + i),
                    new_line_no: None,
                    old_text: Some(format!("deleted {}-{}-{}", path, idx, i)),
                    new_text: None,
                    tokens: vec![],
                });
            }
            old_line += size;
            push_context(&mut lines, &mut old_line, &mut new_line);
        }

        for (idx, &size) in added_sizes.iter().enumerate() {
            for i in 0..size {
                lines.push(DiffLine {
                    kind: LineKind::Added,
                    old_line_no: None,
                    new_line_no: Some(new_line + i),
                    old_text: None,
                    new_text: Some(format!("added {}-{}-{}", path, idx, i)),
                    tokens: vec![],
                });
            }
            new_line += size;
            push_context(&mut lines, &mut old_line, &mut new_line);
        }

        let old_lines = old_line;
        let new_lines = new_line;

        FileDiff {
            path: PathBuf::from(path),
            old_path: None,
            status: FileStatus::Modified,
            hunks: vec![Hunk {
                old_start: 1,
                new_start: 1,
                old_lines,
                new_lines,
                lines,
            }],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }
    }

    #[test]
    fn test_move_candidate_generation_prunes_size_incompatible_blocks() {
        let deleted_sizes: Vec<usize> = vec![3; 200];
        let added_sizes: Vec<usize> = vec![20; 200];
        let file = build_synthetic_file_with_block_sizes(
            "src/huge.rs",
            &deleted_sizes,
            &added_sizes,
        );

        let blocks = extract_blocks(&[file]);
        let deleted_blocks: Vec<(usize, &Block)> = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.kind == LineKind::Deleted)
            .collect();
        let added_blocks: Vec<(usize, &Block)> = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.kind == LineKind::Added)
            .collect();

        let candidates = build_move_candidates(&deleted_blocks, &added_blocks);

        assert_eq!(candidates.len(), 0);
        assert!(
            !deleted_blocks.is_empty() && !added_blocks.is_empty(),
            "expected non-empty move blocks"
        );
        assert!(
            deleted_blocks.len() * added_blocks.len() > 0,
            "expected positive full-pair baseline"
        );
    }

    #[test]
    fn test_detect_move_renamed_file_uses_old_path_for_source() {
        let moved_lines = ["fn moved_for_rename() {", "    let value = 1;", "}"];

        let mut deleted_lines = moved_lines
            .iter()
            .enumerate()
            .map(|(i, text)| DiffLine {
                kind: LineKind::Deleted,
                old_line_no: Some(10 + i),
                new_line_no: None,
                old_text: Some((*text).to_string()),
                new_text: None,
                tokens: vec![],
            })
            .collect::<Vec<_>>();

        deleted_lines.push(DiffLine {
            kind: LineKind::Context,
            old_line_no: Some(13),
            new_line_no: Some(13),
            old_text: Some("// separator".to_string()),
            new_text: Some("// separator".to_string()),
            tokens: vec![],
        });

        let mut added_lines = moved_lines
            .iter()
            .enumerate()
            .map(|(i, text)| DiffLine {
                kind: LineKind::Added,
                old_line_no: None,
                new_text: Some((*text).to_string()),
                old_text: None,
                new_line_no: Some(110 + i),
                tokens: vec![],
            })
            .collect::<Vec<_>>();

        let mut lines = deleted_lines;
        lines.append(&mut added_lines);

        let mut files = vec![FileDiff {
            path: PathBuf::from("src/new_name.rs"),
            old_path: Some(PathBuf::from("src/old_name.rs")),
            status: FileStatus::Renamed,
            hunks: vec![Hunk {
                old_start: 10,
                new_start: 110,
                old_lines: 4,
                new_lines: 4,
                lines,
            }],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }];

        detect_moves(&mut files);

        assert_eq!(files[0].move_matches.len(), 1);
        assert_eq!(files[0].move_matches[0].source_file, PathBuf::from("src/old_name.rs"));
        assert_eq!(files[0].move_matches[0].dest_file, PathBuf::from("src/new_name.rs"));
    }
}
