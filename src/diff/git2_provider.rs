use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{Delta, DiffOptions, Repository};

use super::folding::compute_fold_regions;
use super::model::{DiffLine, DiffMode, FileDiff, FileStatus, Hunk, LineKind};
use super::moves::detect_moves;
use super::provider::DiffProvider;
use super::tokens::compute_token_changes;

pub struct Git2Provider;

impl Git2Provider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Git2Provider {
    fn default() -> Self {
        Self::new()
    }
}

struct RawFile {
    path: PathBuf,
    status: FileStatus,
    hunks: Vec<RawHunk>,
}

struct RawHunk {
    old_start: usize,
    new_start: usize,
    old_lines: usize,
    new_lines: usize,
    lines: Vec<RawLine>,
}

struct RawLine {
    kind: LineKind,
    old_line_no: Option<usize>,
    new_line_no: Option<usize>,
    content: String,
}

impl DiffProvider for Git2Provider {
    fn compute_diff(&self, repo_path: &Path, mode: DiffMode) -> Result<Vec<FileDiff>> {
        let repo =
            Repository::discover(repo_path).context("Failed to discover git repository")?;

        let mut opts = DiffOptions::new();
        opts.context_lines(3);

        let diff = match mode {
            DiffMode::WorkingTree => {
                let head_tree = repo
                    .head()
                    .ok()
                    .and_then(|head| head.peel_to_tree().ok());
                repo.diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
                    .context("Failed to compute working tree diff")?
            }
            DiffMode::Staged => {
                let head_tree = repo
                    .head()
                    .ok()
                    .and_then(|head| head.peel_to_tree().ok());
                let index = repo.index().context("Failed to read index")?;
                repo.diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut opts))
                    .context("Failed to compute staged diff")?
            }
        };

        // Use RefCell to allow shared mutable access across callbacks
        let raw_files: RefCell<Vec<RawFile>> = RefCell::new(Vec::new());

        diff.foreach(
            &mut |delta, _progress| {
                let path = delta
                    .new_file()
                    .path()
                    .or_else(|| delta.old_file().path())
                    .unwrap_or(Path::new(""))
                    .to_path_buf();

                let status = match delta.status() {
                    Delta::Added => FileStatus::Added,
                    Delta::Deleted => FileStatus::Deleted,
                    Delta::Renamed => FileStatus::Renamed,
                    _ => FileStatus::Modified,
                };

                raw_files.borrow_mut().push(RawFile {
                    path,
                    status,
                    hunks: Vec::new(),
                });
                true
            },
            None, // binary callback
            Some(&mut |_delta, hunk| {
                let mut files = raw_files.borrow_mut();
                if let Some(file) = files.last_mut() {
                    file.hunks.push(RawHunk {
                        old_start: hunk.old_start() as usize,
                        new_start: hunk.new_start() as usize,
                        old_lines: hunk.old_lines() as usize,
                        new_lines: hunk.new_lines() as usize,
                        lines: Vec::new(),
                    });
                }
                true
            }),
            Some(&mut |_delta, _hunk, line| {
                let kind = match line.origin() {
                    '+' => LineKind::Added,
                    '-' => LineKind::Deleted,
                    ' ' => LineKind::Context,
                    _ => return true,
                };

                let content = String::from_utf8_lossy(line.content()).to_string();

                let (old_line_no, new_line_no) = match kind {
                    LineKind::Added => (None, line.new_lineno().map(|n| n as usize)),
                    LineKind::Deleted => (line.old_lineno().map(|n| n as usize), None),
                    LineKind::Context => (
                        line.old_lineno().map(|n| n as usize),
                        line.new_lineno().map(|n| n as usize),
                    ),
                    _ => (None, None),
                };

                let mut files = raw_files.borrow_mut();
                if let Some(file) = files.last_mut()
                    && let Some(hunk) = file.hunks.last_mut()
                {
                    hunk.lines.push(RawLine {
                        kind,
                        old_line_no,
                        new_line_no,
                        content,
                    });
                }
                true
            }),
        )
        .context("Failed to iterate diff")?;

        // Build FileDiff structs from raw data, loading file contents
        let mut file_diffs: Vec<FileDiff> = Vec::new();
        for raw_file in raw_files.into_inner() {
            let mut hunks: Vec<Hunk> = raw_file
                .hunks
                .into_iter()
                .map(|raw_hunk| {
                    let lines: Vec<DiffLine> = raw_hunk
                        .lines
                        .into_iter()
                        .map(|raw_line| {
                            let (old_text, new_text) = match raw_line.kind {
                                LineKind::Context => (
                                    Some(raw_line.content.clone()),
                                    Some(raw_line.content.clone()),
                                ),
                                LineKind::Added => (None, Some(raw_line.content.clone())),
                                LineKind::Deleted => (Some(raw_line.content.clone()), None),
                                LineKind::Modified => (
                                    Some(raw_line.content.clone()),
                                    Some(raw_line.content.clone()),
                                ),
                            };

                            DiffLine {
                                kind: raw_line.kind,
                                old_line_no: raw_line.old_line_no,
                                new_line_no: raw_line.new_line_no,
                                old_text,
                                new_text,
                                tokens: Vec::new(),
                            }
                        })
                        .collect();

                    Hunk {
                        old_start: raw_hunk.old_start,
                        new_start: raw_hunk.new_start,
                        old_lines: raw_hunk.old_lines,
                        new_lines: raw_hunk.new_lines,
                        lines,
                    }
                })
                .collect();

            // Compute word-level token diffs for adjacent deleted+added pairs
            for hunk in &mut hunks {
                compute_line_tokens(hunk);
            }

            // Load file contents
            let old_content = load_old_content(&repo, &raw_file.path).unwrap_or_default();
            let new_content = if mode == DiffMode::Staged && raw_file.status != FileStatus::Deleted
            {
                load_staged_content(&repo, &raw_file.path).unwrap_or_default()
            } else {
                load_new_content(&repo, &raw_file.path).unwrap_or_default()
            };

            file_diffs.push(FileDiff {
                path: raw_file.path,
                status: raw_file.status,
                hunks,
                old_content,
                new_content,
                fold_regions: Vec::new(),
                move_matches: Vec::new(),
            });
        }

        // Compute structural fold regions from file contents
        for file_diff in &mut file_diffs {
            if !file_diff.new_content.is_empty() {
                file_diff.fold_regions = compute_fold_regions(&file_diff.new_content);
            } else if !file_diff.old_content.is_empty() {
                file_diff.fold_regions = compute_fold_regions(&file_diff.old_content);
            }
        }

        // Detect moved blocks across files
        detect_moves(&mut file_diffs);

        Ok(file_diffs)
    }
}

/// Load the old (HEAD) version of a file from the repository.
fn load_old_content(repo: &Repository, path: &Path) -> Result<String> {
    let head = repo.head().context("Failed to get HEAD")?;
    let tree = head.peel_to_tree().context("Failed to peel HEAD to tree")?;
    let entry = tree
        .get_path(path)
        .context("File not found in HEAD tree")?;
    let blob = repo
        .find_blob(entry.id())
        .context("Failed to find blob")?;
    let content = String::from_utf8_lossy(blob.content()).to_string();
    Ok(content)
}

/// Load the new (working tree) version of a file from the filesystem.
fn load_new_content(repo: &Repository, path: &Path) -> Result<String> {
    let workdir = repo
        .workdir()
        .context("Repository has no working directory")?;
    let full_path = workdir.join(path);
    let content = std::fs::read_to_string(&full_path)
        .with_context(|| format!("Failed to read file: {}", full_path.display()))?;
    Ok(content)
}

/// Load the staged (index) version of a file from the git index.
fn load_staged_content(repo: &Repository, path: &Path) -> Result<String> {
    let index = repo.index().context("Failed to read index")?;
    let entry = index
        .get_path(path, 0)
        .context("File not found in index")?;
    let blob = repo
        .find_blob(entry.id)
        .context("Failed to find blob for index entry")?;
    let content = std::str::from_utf8(blob.content())
        .unwrap_or_default()
        .to_string();
    Ok(content)
}

/// Compute word-level token changes for contiguous blocks of Deleted+Added lines
/// in a hunk, pairing them 1:1 up to min(N, M) and promoting paired lines to Modified.
fn compute_line_tokens(hunk: &mut Hunk) {
    let len = hunk.lines.len();
    let mut i = 0;
    while i < len {
        // Find a contiguous block of Deleted lines
        if hunk.lines[i].kind != LineKind::Deleted {
            i += 1;
            continue;
        }

        let del_start = i;
        while i < len && hunk.lines[i].kind == LineKind::Deleted {
            i += 1;
        }
        let del_end = i; // exclusive
        let del_count = del_end - del_start;

        // Find a contiguous block of Added lines immediately after
        let add_start = i;
        while i < len && hunk.lines[i].kind == LineKind::Added {
            i += 1;
        }
        let add_end = i; // exclusive
        let add_count = add_end - add_start;

        if add_count == 0 {
            // No added lines following the deleted block; nothing to pair
            continue;
        }

        // Pair them 1:1 up to min(del_count, add_count)
        let pairs = del_count.min(add_count);
        for p in 0..pairs {
            let del_idx = del_start + p;
            let add_idx = add_start + p;

            let old_text = hunk.lines[del_idx]
                .old_text
                .clone()
                .unwrap_or_default();
            let new_text = hunk.lines[add_idx]
                .new_text
                .clone()
                .unwrap_or_default();

            let (old_tokens, new_tokens) = compute_token_changes(&old_text, &new_text);

            // Promote the deleted line to Modified, storing old-side tokens
            hunk.lines[del_idx].kind = LineKind::Modified;
            hunk.lines[del_idx].new_line_no = hunk.lines[add_idx].new_line_no;
            hunk.lines[del_idx].new_text = Some(new_text.clone());
            hunk.lines[del_idx].tokens = old_tokens;

            // Promote the added line to Modified, storing new-side tokens
            hunk.lines[add_idx].kind = LineKind::Modified;
            hunk.lines[add_idx].old_line_no = hunk.lines[del_idx].old_line_no;
            hunk.lines[add_idx].old_text = Some(old_text);
            hunk.lines[add_idx].tokens = new_tokens;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Creates a temporary git repo with an initial commit containing a single file.
    /// Returns the TempDir (must be kept alive) and the path to the repo directory.
    fn setup_test_repo() -> (tempfile::TempDir, PathBuf) {
        let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let repo_path = tmp_dir.path().to_path_buf();

        let repo = Repository::init(&repo_path).expect("Failed to init repo");

        // Configure user for commits
        let mut config = repo.config().expect("Failed to get config");
        config
            .set_str("user.name", "Test User")
            .expect("Failed to set user name");
        config
            .set_str("user.email", "test@example.com")
            .expect("Failed to set user email");

        // Create initial file
        let file_path = repo_path.join("hello.txt");
        fs::write(&file_path, "hello world\n").expect("Failed to write file");

        // Stage and commit
        let mut index = repo.index().expect("Failed to get index");
        index
            .add_path(Path::new("hello.txt"))
            .expect("Failed to add file");
        index.write().expect("Failed to write index");

        let tree_id = index.write_tree().expect("Failed to write tree");
        let tree = repo.find_tree(tree_id).expect("Failed to find tree");
        let sig = repo.signature().expect("Failed to create signature");

        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .expect("Failed to create commit");

        (tmp_dir, repo_path)
    }

    #[test]
    fn test_no_changes() {
        let (_tmp_dir, repo_path) = setup_test_repo();
        let provider = Git2Provider::new();

        let diffs = provider
            .compute_diff(&repo_path, DiffMode::WorkingTree)
            .expect("Failed to compute diff");

        assert!(diffs.is_empty(), "Expected no diffs for clean repo");
    }

    #[test]
    fn test_working_tree_modification() {
        let (_tmp_dir, repo_path) = setup_test_repo();
        let provider = Git2Provider::new();

        // Modify the file
        let file_path = repo_path.join("hello.txt");
        fs::write(&file_path, "hello world\ngoodbye world\n").expect("Failed to write file");

        let diffs = provider
            .compute_diff(&repo_path, DiffMode::WorkingTree)
            .expect("Failed to compute diff");

        assert_eq!(diffs.len(), 1, "Expected exactly one file diff");

        let diff = &diffs[0];
        assert_eq!(diff.path, PathBuf::from("hello.txt"));
        assert_eq!(diff.status, FileStatus::Modified);
        assert!(!diff.hunks.is_empty(), "Expected at least one hunk");

        // Check that the hunk contains an added line
        let hunk = &diff.hunks[0];
        let has_added_line = hunk.lines.iter().any(|l| l.kind == LineKind::Added);
        assert!(has_added_line, "Expected an added line in the hunk");
    }

    #[test]
    fn test_staged_changes() {
        let (_tmp_dir, repo_path) = setup_test_repo();
        let provider = Git2Provider::new();

        // Modify and stage the file
        let file_path = repo_path.join("hello.txt");
        fs::write(&file_path, "hello world\nstaged line\n").expect("Failed to write file");

        let repo = Repository::open(&repo_path).expect("Failed to open repo");
        let mut index = repo.index().expect("Failed to get index");
        index
            .add_path(Path::new("hello.txt"))
            .expect("Failed to stage file");
        index.write().expect("Failed to write index");

        let diffs = provider
            .compute_diff(&repo_path, DiffMode::Staged)
            .expect("Failed to compute diff");

        assert_eq!(diffs.len(), 1, "Expected exactly one staged file diff");
        assert_eq!(diffs[0].status, FileStatus::Modified);
        assert_eq!(diffs[0].path, PathBuf::from("hello.txt"));
        assert!(
            !diffs[0].hunks.is_empty(),
            "Expected at least one hunk in staged diff"
        );
    }

    #[test]
    fn test_added_file() {
        let (_tmp_dir, repo_path) = setup_test_repo();
        let provider = Git2Provider::new();

        // Create a new file (untracked files show up in working tree diff with index)
        let new_file = repo_path.join("new_file.txt");
        fs::write(&new_file, "brand new content\n").expect("Failed to write new file");

        // Stage the file so it appears in diff_tree_to_workdir_with_index
        let repo = Repository::open(&repo_path).expect("Failed to open repo");
        let mut index = repo.index().expect("Failed to get index");
        index
            .add_path(Path::new("new_file.txt"))
            .expect("Failed to add new file");
        index.write().expect("Failed to write index");

        let diffs = provider
            .compute_diff(&repo_path, DiffMode::WorkingTree)
            .expect("Failed to compute diff");

        let new_file_diff = diffs
            .iter()
            .find(|d| d.path == PathBuf::from("new_file.txt"));
        assert!(
            new_file_diff.is_some(),
            "Expected to find the new file in diffs"
        );
        assert_eq!(new_file_diff.unwrap().status, FileStatus::Added);
    }

    #[test]
    fn test_file_contents_loaded() {
        let (_tmp_dir, repo_path) = setup_test_repo();
        let provider = Git2Provider::new();

        // Modify the file
        let file_path = repo_path.join("hello.txt");
        fs::write(&file_path, "hello world\nextra line\n").expect("Failed to write file");

        let diffs = provider
            .compute_diff(&repo_path, DiffMode::WorkingTree)
            .expect("Failed to compute diff");

        assert_eq!(diffs.len(), 1);
        let diff = &diffs[0];

        assert_eq!(
            diff.old_content, "hello world\n",
            "Old content should match the original committed file"
        );
        assert_eq!(
            diff.new_content, "hello world\nextra line\n",
            "New content should match the modified file on disk"
        );
    }
}
