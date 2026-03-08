# better-diff Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Rust TUI git diff viewer with token-level highlighting, structural folding, move detection, heat map minimap, and change animations.

**Architecture:** Three decoupled layers — input (notify + CLI), diff engine (git2 + similar + tree-sitter behind a `DiffProvider` trait), and TUI (ratatui). Single-threaded event loop with crossbeam-channel select over filesystem events, keyboard input, and tick timer.

**Tech Stack:** Rust, ratatui 0.30, git2 0.20, similar, tree-sitter, notify 8.x, crossbeam-channel, clap 4.x, anyhow

---

### Task 1: Project Scaffolding

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`

**Step 1: Initialize Cargo project**

Run: `cargo init --name better-diff`

**Step 2: Set up Cargo.toml with all dependencies**

```toml
[package]
name = "better-diff"
version = "0.1.0"
edition = "2024"

[dependencies]
ratatui = "0.30"
crossterm = "0.28"
git2 = "0.20"
similar = { version = "2", features = ["inline"] }
tree-sitter = "0.25"
tree-sitter-rust = "0.23"
notify = "8"
notify-debouncer-mini = "0.6"
crossbeam-channel = "0.5"
clap = { version = "4", features = ["derive"] }
anyhow = "1"
```

**Step 3: Write minimal main.rs that compiles**

```rust
use anyhow::Result;

fn main() -> Result<()> {
    println!("better-diff");
    Ok(())
}
```

**Step 4: Verify it compiles**

Run: `cargo build`
Expected: Compiles successfully (dependencies download on first build)

**Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs
git commit -m "feat: initialize project with dependencies"
```

---

### Task 2: Data Model

**Files:**
- Create: `src/diff/mod.rs`
- Create: `src/diff/model.rs`

**Step 1: Write tests for the data model**

In `src/diff/model.rs`:

```rust
use std::ops::Range;
use std::path::PathBuf;

/// The two diff modes the app supports
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    /// Working tree vs HEAD
    WorkingTree,
    /// Staged (index) vs HEAD
    Staged,
}

/// Status of a file in the diff
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
}

/// What kind of diff line this is
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Unchanged context line
    Context,
    /// Line was added
    Added,
    /// Line was deleted
    Deleted,
    /// Line was modified (has both old and new text)
    Modified,
}

/// Type of token-level change within a line
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Unchanged token
    Equal,
    /// Token was replaced with something else
    Rename,
    /// Token was inserted
    Addition,
    /// Token was removed
    Deletion,
}

/// A single token-level change within a diff line
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenChange {
    pub kind: ChangeKind,
    pub text: String,
}

/// A single line in a diff hunk
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_line_no: Option<usize>,
    pub new_line_no: Option<usize>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub tokens: Vec<TokenChange>,
}

/// A contiguous group of diff lines
#[derive(Debug, Clone)]
pub struct Hunk {
    pub old_start: usize,
    pub new_start: usize,
    pub old_lines: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
}

/// What kind of fold region this is
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldKind {
    Function,
    Impl,
    Module,
    Struct,
    Enum,
    Other,
}

/// A region that can be collapsed
#[derive(Debug, Clone)]
pub struct FoldRegion {
    pub kind: FoldKind,
    pub label: String,
    pub old_start: usize,
    pub old_end: usize,
    pub new_start: usize,
    pub new_end: usize,
    pub is_collapsed: bool,
}

/// A detected block move
#[derive(Debug, Clone)]
pub struct MoveMatch {
    pub source_file: PathBuf,
    pub source_start: usize,
    pub source_end: usize,
    pub dest_file: PathBuf,
    pub dest_start: usize,
    pub dest_end: usize,
    pub similarity: f64,
}

/// All diff data for a single file
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: PathBuf,
    pub status: FileStatus,
    pub hunks: Vec<Hunk>,
    pub old_content: String,
    pub new_content: String,
    pub fold_regions: Vec<FoldRegion>,
    pub move_matches: Vec<MoveMatch>,
}

/// Collapse level for folding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapseLevel {
    /// Only changed lines + 3 lines context
    Tight,
    /// Collapse by AST scope
    Scoped,
    /// Show everything
    Expanded,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_line_context() {
        let line = DiffLine {
            kind: LineKind::Context,
            old_line_no: Some(1),
            new_line_no: Some(1),
            old_text: Some("let x = 1;".into()),
            new_text: Some("let x = 1;".into()),
            tokens: vec![],
        };
        assert_eq!(line.kind, LineKind::Context);
        assert_eq!(line.old_line_no, Some(1));
    }

    #[test]
    fn test_diff_line_modified_with_tokens() {
        let line = DiffLine {
            kind: LineKind::Modified,
            old_line_no: Some(5),
            new_line_no: Some(5),
            old_text: Some("let x = foo(a, b);".into()),
            new_text: Some("let x = bar(a, b, c);".into()),
            tokens: vec![
                TokenChange { kind: ChangeKind::Equal, text: "let x = ".into() },
                TokenChange { kind: ChangeKind::Rename, text: "foo".into() },
                TokenChange { kind: ChangeKind::Equal, text: "(a, b".into() },
                TokenChange { kind: ChangeKind::Addition, text: ", c".into() },
                TokenChange { kind: ChangeKind::Equal, text: ");".into() },
            ],
        };
        assert_eq!(line.tokens.len(), 5);
        assert_eq!(line.tokens[1].kind, ChangeKind::Rename);
        assert_eq!(line.tokens[3].kind, ChangeKind::Addition);
    }

    #[test]
    fn test_file_diff_creation() {
        let diff = FileDiff {
            path: PathBuf::from("src/main.rs"),
            status: FileStatus::Modified,
            hunks: vec![],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        };
        assert_eq!(diff.status, FileStatus::Modified);
        assert_eq!(diff.path, PathBuf::from("src/main.rs"));
    }

    #[test]
    fn test_collapse_levels() {
        assert_ne!(CollapseLevel::Tight, CollapseLevel::Scoped);
        assert_ne!(CollapseLevel::Scoped, CollapseLevel::Expanded);
    }
}
```

**Step 2: Create diff module**

In `src/diff/mod.rs`:

```rust
pub mod model;
```

**Step 3: Wire into main.rs**

```rust
mod diff;

use anyhow::Result;

fn main() -> Result<()> {
    println!("better-diff");
    Ok(())
}
```

**Step 4: Run tests**

Run: `cargo test`
Expected: All 4 tests pass

**Step 5: Commit**

```bash
git add src/diff/
git commit -m "feat: add diff data model with types and tests"
```

---

### Task 3: DiffProvider Trait & Git2 Line-Level Diffing

**Files:**
- Modify: `src/diff/mod.rs`
- Create: `src/diff/provider.rs`
- Create: `src/diff/git2_provider.rs`

**Step 1: Define the DiffProvider trait**

In `src/diff/provider.rs`:

```rust
use anyhow::Result;
use std::path::Path;

use super::model::{DiffMode, FileDiff};

/// Trait for computing diffs from a git repository
pub trait DiffProvider {
    fn compute_diff(&self, repo_path: &Path, mode: DiffMode) -> Result<Vec<FileDiff>>;
}
```

**Step 2: Implement Git2Provider**

In `src/diff/git2_provider.rs`:

```rust
use anyhow::{Context, Result};
use git2::{Delta, DiffOptions, Repository};
use std::path::Path;

use super::model::*;
use super::provider::DiffProvider;

pub struct Git2Provider;

impl Git2Provider {
    pub fn new() -> Self {
        Self
    }
}

impl DiffProvider for Git2Provider {
    fn compute_diff(&self, repo_path: &Path, mode: DiffMode) -> Result<Vec<FileDiff>> {
        let repo = Repository::discover(repo_path)
            .context("Failed to open git repository")?;

        let mut opts = DiffOptions::new();
        opts.context_lines(3);

        let diff = match mode {
            DiffMode::WorkingTree => {
                // Working tree vs HEAD
                let head = repo.head()?.peel_to_tree()?;
                repo.diff_tree_to_workdir_with_index(Some(&head), Some(&mut opts))?
            }
            DiffMode::Staged => {
                // Staged (index) vs HEAD
                let head = repo.head()?.peel_to_tree()?;
                repo.diff_tree_to_index(Some(&head), None, Some(&mut opts))?
            }
        };

        let mut file_diffs: Vec<FileDiff> = Vec::new();

        diff.foreach(
            &mut |delta, _progress| {
                let path = delta
                    .new_file()
                    .path()
                    .or_else(|| delta.old_file().path())
                    .unwrap_or(Path::new("unknown"))
                    .to_path_buf();

                let status = match delta.status() {
                    Delta::Added => FileStatus::Added,
                    Delta::Deleted => FileStatus::Deleted,
                    Delta::Renamed => FileStatus::Renamed,
                    _ => FileStatus::Modified,
                };

                file_diffs.push(FileDiff {
                    path,
                    status,
                    hunks: vec![],
                    old_content: String::new(),
                    new_content: String::new(),
                    fold_regions: vec![],
                    move_matches: vec![],
                });
                true
            },
            None, // binary callback
            Some(&mut |_delta, hunk| {
                if let Some(file) = file_diffs.last_mut() {
                    file.hunks.push(Hunk {
                        old_start: hunk.old_start() as usize,
                        new_start: hunk.new_start() as usize,
                        old_lines: hunk.old_lines() as usize,
                        new_lines: hunk.new_lines() as usize,
                        lines: vec![],
                    });
                }
                true
            }),
            Some(&mut |_delta, _hunk, line| {
                if let Some(file) = file_diffs.last_mut() {
                    if let Some(hunk) = file.hunks.last_mut() {
                        let content = std::str::from_utf8(line.content())
                            .unwrap_or("")
                            .to_string();

                        let (kind, old_text, new_text, old_line_no, new_line_no) =
                            match line.origin() {
                                '+' => (
                                    LineKind::Added,
                                    None,
                                    Some(content),
                                    None,
                                    line.new_lineno().map(|n| n as usize),
                                ),
                                '-' => (
                                    LineKind::Deleted,
                                    Some(content),
                                    None,
                                    line.old_lineno().map(|n| n as usize),
                                    None,
                                ),
                                _ => (
                                    LineKind::Context,
                                    Some(content.clone()),
                                    Some(content),
                                    line.old_lineno().map(|n| n as usize),
                                    line.new_lineno().map(|n| n as usize),
                                ),
                            };

                        hunk.lines.push(DiffLine {
                            kind,
                            old_line_no,
                            new_line_no,
                            old_text,
                            new_text,
                            tokens: vec![],
                        });
                    }
                }
                true
            }),
        )?;

        // Load file contents for each diff
        for file_diff in &mut file_diffs {
            let workdir = repo.workdir().unwrap_or(repo_path);
            let full_path = workdir.join(&file_diff.path);

            // New content from working directory
            if file_diff.status != FileStatus::Deleted {
                file_diff.new_content =
                    std::fs::read_to_string(&full_path).unwrap_or_default();
            }

            // Old content from HEAD
            if file_diff.status != FileStatus::Added {
                if let Ok(head) = repo.head() {
                    if let Ok(tree) = head.peel_to_tree() {
                        if let Ok(entry) = tree.get_path(&file_diff.path) {
                            if let Ok(blob) = repo.find_blob(entry.id()) {
                                file_diff.old_content =
                                    std::str::from_utf8(blob.content())
                                        .unwrap_or_default()
                                        .to_string();
                            }
                        }
                    }
                }
            }
        }

        Ok(file_diffs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn setup_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();

        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(path)
            .output()
            .unwrap();

        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .output()
            .unwrap();

        // Create initial file and commit
        std::fs::write(path.join("test.txt"), "line1\nline2\nline3\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(path)
            .output()
            .unwrap();

        dir
    }

    #[test]
    fn test_no_changes() {
        let dir = setup_test_repo();
        let provider = Git2Provider::new();
        let result = provider
            .compute_diff(dir.path(), DiffMode::WorkingTree)
            .unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_working_tree_modification() {
        let dir = setup_test_repo();

        // Modify the file
        std::fs::write(dir.path().join("test.txt"), "line1\nmodified\nline3\n").unwrap();

        let provider = Git2Provider::new();
        let result = provider
            .compute_diff(dir.path(), DiffMode::WorkingTree)
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, PathBuf::from("test.txt"));
        assert_eq!(result[0].status, FileStatus::Modified);
        assert!(!result[0].hunks.is_empty());
    }

    #[test]
    fn test_staged_changes() {
        let dir = setup_test_repo();

        // Modify and stage
        std::fs::write(dir.path().join("test.txt"), "line1\nstaged change\nline3\n").unwrap();
        Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let provider = Git2Provider::new();
        let result = provider
            .compute_diff(dir.path(), DiffMode::Staged)
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Modified);
    }

    #[test]
    fn test_added_file() {
        let dir = setup_test_repo();

        std::fs::write(dir.path().join("new.txt"), "new file content\n").unwrap();

        let provider = Git2Provider::new();
        let result = provider
            .compute_diff(dir.path(), DiffMode::WorkingTree)
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Added);
    }

    #[test]
    fn test_file_contents_loaded() {
        let dir = setup_test_repo();

        std::fs::write(dir.path().join("test.txt"), "line1\nchanged\nline3\n").unwrap();

        let provider = Git2Provider::new();
        let result = provider
            .compute_diff(dir.path(), DiffMode::WorkingTree)
            .unwrap();

        assert_eq!(result[0].old_content, "line1\nline2\nline3\n");
        assert_eq!(result[0].new_content, "line1\nchanged\nline3\n");
    }
}
```

**Step 3: Add tempfile as dev dependency**

Add to `Cargo.toml`:

```toml
[dev-dependencies]
tempfile = "3"
```

**Step 4: Update diff/mod.rs**

```rust
pub mod model;
pub mod provider;
pub mod git2_provider;
```

**Step 5: Run tests**

Run: `cargo test`
Expected: All tests pass

**Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/diff/
git commit -m "feat: add DiffProvider trait and git2 line-level diffing"
```

---

### Task 4: Word-Level Token Diffing with `similar`

**Files:**
- Create: `src/diff/tokens.rs`
- Modify: `src/diff/mod.rs`
- Modify: `src/diff/git2_provider.rs`

**Step 1: Write token diff module with tests**

In `src/diff/tokens.rs`:

```rust
use similar::{ChangeTag, TextDiff};

use super::model::{ChangeKind, TokenChange};

/// Compute word-level token changes between two strings.
/// Returns a list of TokenChanges for the old side and new side.
pub fn compute_token_changes(old: &str, new: &str) -> (Vec<TokenChange>, Vec<TokenChange>) {
    let diff = TextDiff::from_words(old, new);
    let mut old_tokens = Vec::new();
    let mut new_tokens = Vec::new();

    for change in diff.iter_all_changes() {
        let text = change.value().to_string();
        match change.tag() {
            ChangeTag::Equal => {
                old_tokens.push(TokenChange {
                    kind: ChangeKind::Equal,
                    text: text.clone(),
                });
                new_tokens.push(TokenChange {
                    kind: ChangeKind::Equal,
                    text,
                });
            }
            ChangeTag::Delete => {
                old_tokens.push(TokenChange {
                    kind: ChangeKind::Deletion,
                    text,
                });
            }
            ChangeTag::Insert => {
                new_tokens.push(TokenChange {
                    kind: ChangeKind::Addition,
                    text,
                });
            }
        }
    }

    // Post-process: detect renames (adjacent delete+insert of similar length)
    promote_renames(&mut old_tokens, &mut new_tokens);

    (old_tokens, new_tokens)
}

/// Detect adjacent delete/insert pairs and promote them to renames.
fn promote_renames(old_tokens: &mut [TokenChange], new_tokens: &mut [TokenChange]) {
    let deletions: Vec<usize> = old_tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == ChangeKind::Deletion)
        .map(|(i, _)| i)
        .collect();

    let additions: Vec<usize> = new_tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == ChangeKind::Addition)
        .map(|(i, _)| i)
        .collect();

    // Match deletions with additions in order
    for (del_idx, add_idx) in deletions.iter().zip(additions.iter()) {
        let del = &old_tokens[*del_idx];
        let add = &new_tokens[*add_idx];

        // Both are non-whitespace single tokens — likely a rename
        if !del.text.trim().is_empty() && !add.text.trim().is_empty() {
            old_tokens[*del_idx].kind = ChangeKind::Rename;
            new_tokens[*add_idx].kind = ChangeKind::Rename;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_strings() {
        let (old, new) = compute_token_changes("let x = 1;", "let x = 1;");
        assert!(old.iter().all(|t| t.kind == ChangeKind::Equal));
        assert!(new.iter().all(|t| t.kind == ChangeKind::Equal));
    }

    #[test]
    fn test_rename_detection() {
        let (old, new) = compute_token_changes("foo(a, b)", "bar(a, b)");

        let old_rename = old.iter().find(|t| t.kind == ChangeKind::Rename);
        assert!(old_rename.is_some());
        assert_eq!(old_rename.unwrap().text, "foo");

        let new_rename = new.iter().find(|t| t.kind == ChangeKind::Rename);
        assert!(new_rename.is_some());
        assert_eq!(new_rename.unwrap().text, "bar");
    }

    #[test]
    fn test_addition_detection() {
        let (old, new) = compute_token_changes("foo(a, b)", "foo(a, b, c)");

        let has_addition = new.iter().any(|t| t.kind == ChangeKind::Addition);
        assert!(has_addition);

        // Old side should have no additions
        let old_has_addition = old.iter().any(|t| t.kind == ChangeKind::Addition);
        assert!(!old_has_addition);
    }

    #[test]
    fn test_deletion_detection() {
        let (old, new) = compute_token_changes("foo(a, b, c)", "foo(a, b)");

        let has_deletion = old.iter().any(|t| t.kind == ChangeKind::Deletion);
        assert!(has_deletion);
    }

    #[test]
    fn test_complex_change() {
        let (old, new) = compute_token_changes(
            "let result = process(data);",
            "let output = transform(data, opts);",
        );

        // "result" -> "output" should be a rename
        let old_renames: Vec<_> = old.iter().filter(|t| t.kind == ChangeKind::Rename).collect();
        let new_renames: Vec<_> = new.iter().filter(|t| t.kind == ChangeKind::Rename).collect();
        assert!(!old_renames.is_empty());
        assert!(!new_renames.is_empty());
    }
}
```

**Step 2: Integrate token diffing into git2_provider**

Add to `src/diff/git2_provider.rs`, after the line-level diff loop, before returning `file_diffs`:

```rust
// After loading file contents, compute token-level diffs for modified lines
for file_diff in &mut file_diffs {
    for hunk in &mut file_diff.hunks {
        compute_line_tokens(hunk);
    }
}
```

Add this function to `src/diff/git2_provider.rs`:

```rust
use super::tokens::compute_token_changes;

/// Pair up adjacent deleted+added lines in a hunk and compute token diffs.
fn compute_line_tokens(hunk: &mut Hunk) {
    let mut i = 0;
    while i < hunk.lines.len() {
        if hunk.lines[i].kind == LineKind::Deleted {
            // Look for a matching Added line immediately after
            let mut j = i + 1;
            while j < hunk.lines.len() && hunk.lines[j].kind == LineKind::Deleted {
                j += 1;
            }

            // Pair deleted lines with added lines
            let del_start = i;
            let del_end = j;
            let mut add_end = j;
            while add_end < hunk.lines.len() && hunk.lines[add_end].kind == LineKind::Added {
                add_end += 1;
            }

            let del_count = del_end - del_start;
            let add_count = add_end - del_end;
            let pairs = del_count.min(add_count);

            for p in 0..pairs {
                let del_idx = del_start + p;
                let add_idx = del_end + p;

                let old_text = hunk.lines[del_idx]
                    .old_text
                    .clone()
                    .unwrap_or_default();
                let new_text = hunk.lines[add_idx]
                    .new_text
                    .clone()
                    .unwrap_or_default();

                let (old_tokens, new_tokens) =
                    compute_token_changes(&old_text, &new_text);

                hunk.lines[del_idx].kind = LineKind::Modified;
                hunk.lines[del_idx].new_text = Some(new_text);
                hunk.lines[del_idx].new_line_no = hunk.lines[add_idx].new_line_no;
                hunk.lines[del_idx].tokens = old_tokens;

                hunk.lines[add_idx].kind = LineKind::Modified;
                hunk.lines[add_idx].old_text = Some(old_text);
                hunk.lines[add_idx].old_line_no = hunk.lines[del_idx].old_line_no;
                hunk.lines[add_idx].tokens = new_tokens;
            }

            i = add_end;
        } else {
            i += 1;
        }
    }
}
```

**Step 3: Update diff/mod.rs**

```rust
pub mod model;
pub mod provider;
pub mod git2_provider;
pub mod tokens;
```

**Step 4: Run tests**

Run: `cargo test`
Expected: All tests pass (model + git2_provider + tokens)

**Step 5: Commit**

```bash
git add src/diff/
git commit -m "feat: add word-level token diffing with rename detection"
```

---

### Task 5: App State & CLI Args

**Files:**
- Create: `src/app.rs`
- Modify: `src/main.rs`

**Step 1: Create app state**

In `src/app.rs`:

```rust
use std::path::PathBuf;

use crate::diff::model::*;

pub struct App {
    pub mode: DiffMode,
    pub files: Vec<FileDiff>,
    pub active_file: usize,
    pub collapse_level: CollapseLevel,
    pub scroll_offset: usize,
    pub should_quit: bool,
    pub repo_path: PathBuf,
}

impl App {
    pub fn new(repo_path: PathBuf) -> Self {
        Self {
            mode: DiffMode::WorkingTree,
            files: vec![],
            active_file: 0,
            collapse_level: CollapseLevel::Scoped,
            scroll_offset: 0,
            should_quit: false,
            repo_path,
        }
    }

    pub fn active_file(&self) -> Option<&FileDiff> {
        self.files.get(self.active_file)
    }

    pub fn next_file(&mut self) {
        if !self.files.is_empty() {
            self.active_file = (self.active_file + 1) % self.files.len();
            self.scroll_offset = 0;
        }
    }

    pub fn prev_file(&mut self) {
        if !self.files.is_empty() {
            self.active_file = if self.active_file == 0 {
                self.files.len() - 1
            } else {
                self.active_file - 1
            };
            self.scroll_offset = 0;
        }
    }

    pub fn select_file(&mut self, index: usize) {
        if index < self.files.len() {
            self.active_file = index;
            self.scroll_offset = 0;
        }
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            DiffMode::WorkingTree => DiffMode::Staged,
            DiffMode::Staged => DiffMode::WorkingTree,
        };
    }

    pub fn cycle_collapse(&mut self) {
        self.collapse_level = match self.collapse_level {
            CollapseLevel::Tight => CollapseLevel::Scoped,
            CollapseLevel::Scoped => CollapseLevel::Expanded,
            CollapseLevel::Expanded => CollapseLevel::Tight,
        };
    }

    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    /// Jump to the next hunk in the current file
    pub fn next_hunk(&mut self) {
        if let Some(file) = self.active_file() {
            let mut line_idx = 0;
            let mut found_current = false;

            for hunk in &file.hunks {
                for line in &hunk.lines {
                    if line_idx > self.scroll_offset && !found_current {
                        if line.kind != LineKind::Context {
                            found_current = true;
                        }
                    } else if found_current && line.kind == LineKind::Context {
                        // We've passed the current hunk, look for next
                    }
                    line_idx += 1;
                }
                // Jump to start of next hunk
                if found_current {
                    break;
                }
            }

            // Simple approach: find next hunk start line
            let mut total = 0;
            for hunk in &file.hunks {
                if total > self.scroll_offset {
                    self.scroll_offset = total;
                    return;
                }
                total += hunk.lines.len();
            }
        }
    }

    /// Jump to the previous hunk in the current file
    pub fn prev_hunk(&mut self) {
        if let Some(file) = self.active_file() {
            let mut hunk_starts = vec![];
            let mut total = 0;
            for hunk in &file.hunks {
                hunk_starts.push(total);
                total += hunk.lines.len();
            }

            // Find the last hunk start before current scroll position
            for &start in hunk_starts.iter().rev() {
                if start < self.scroll_offset {
                    self.scroll_offset = start;
                    return;
                }
            }
            self.scroll_offset = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_navigation() {
        let mut app = App::new(PathBuf::from("."));
        app.files = vec![
            FileDiff {
                path: PathBuf::from("a.rs"),
                status: FileStatus::Modified,
                hunks: vec![],
                old_content: String::new(),
                new_content: String::new(),
                fold_regions: vec![],
                move_matches: vec![],
            },
            FileDiff {
                path: PathBuf::from("b.rs"),
                status: FileStatus::Added,
                hunks: vec![],
                old_content: String::new(),
                new_content: String::new(),
                fold_regions: vec![],
                move_matches: vec![],
            },
        ];

        assert_eq!(app.active_file, 0);
        app.next_file();
        assert_eq!(app.active_file, 1);
        app.next_file();
        assert_eq!(app.active_file, 0); // wraps around

        app.prev_file();
        assert_eq!(app.active_file, 1); // wraps backward
    }

    #[test]
    fn test_mode_toggle() {
        let mut app = App::new(PathBuf::from("."));
        assert_eq!(app.mode, DiffMode::WorkingTree);
        app.toggle_mode();
        assert_eq!(app.mode, DiffMode::Staged);
        app.toggle_mode();
        assert_eq!(app.mode, DiffMode::WorkingTree);
    }

    #[test]
    fn test_collapse_cycle() {
        let mut app = App::new(PathBuf::from("."));
        assert_eq!(app.collapse_level, CollapseLevel::Scoped);
        app.cycle_collapse();
        assert_eq!(app.collapse_level, CollapseLevel::Expanded);
        app.cycle_collapse();
        assert_eq!(app.collapse_level, CollapseLevel::Tight);
        app.cycle_collapse();
        assert_eq!(app.collapse_level, CollapseLevel::Scoped);
    }
}
```

**Step 2: Set up CLI args and main.rs**

In `src/main.rs`:

```rust
mod app;
mod diff;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

use app::App;
use diff::git2_provider::Git2Provider;
use diff::model::DiffMode;
use diff::provider::DiffProvider;

#[derive(Parser)]
#[command(name = "better-diff", about = "A better git diff viewer")]
struct Cli {
    /// Path to the git repository
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Start in staged mode
    #[arg(short, long)]
    staged: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut app = App::new(cli.path.clone());
    if cli.staged {
        app.mode = DiffMode::Staged;
    }

    // Compute initial diff
    let provider = Git2Provider::new();
    app.files = provider.compute_diff(&cli.path, app.mode)?;

    println!("Found {} changed files:", app.files.len());
    for file in &app.files {
        println!("  {:?} - {:?}", file.status, file.path);
    }

    Ok(())
}
```

**Step 3: Run tests and verify CLI**

Run: `cargo test`
Expected: All tests pass

Run: `cargo run`
Expected: Prints changed files (or "Found 0 changed files" if clean)

**Step 4: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat: add app state management and CLI argument parsing"
```

---

### Task 6: Basic TUI Shell with Event Loop

**Files:**
- Create: `src/ui/mod.rs`
- Modify: `src/main.rs`

**Step 1: Create the UI module with top-level render**

In `src/ui/mod.rs`:

```rust
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::App;
use crate::diff::model::DiffMode;

pub fn render(frame: &mut Frame, app: &App) {
    let [tabs_area, mode_area, content_area, status_area] = Layout::vertical([
        Constraint::Length(1), // Tab bar
        Constraint::Length(1), // Mode indicator
        Constraint::Fill(1),  // Main content
        Constraint::Length(1), // Status bar
    ])
    .areas(frame.area());

    // --- Tab bar ---
    let tab_titles: Vec<Line> = app
        .files
        .iter()
        .map(|f| {
            let name = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| f.path.to_string_lossy().to_string());
            Line::from(name)
        })
        .collect();

    if !tab_titles.is_empty() {
        let tabs = Tabs::new(tab_titles)
            .select(app.active_file)
            .style(Style::new().fg(Color::DarkGray))
            .highlight_style(
                Style::new()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .divider(" │ ");
        frame.render_widget(tabs, tabs_area);
    } else {
        let empty = Paragraph::new("No changes detected")
            .style(Style::new().fg(Color::DarkGray));
        frame.render_widget(empty, tabs_area);
    }

    // --- Mode indicator ---
    let mode_text = match app.mode {
        DiffMode::WorkingTree => "Working Tree",
        DiffMode::Staged => "Staged",
    };
    let file_count = app.files.len();
    let mode_line = Line::from(vec![
        Span::styled(
            format!(" [{}]", mode_text),
            Style::new().fg(Color::Cyan),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} file{} changed", file_count, if file_count == 1 { "" } else { "s" }),
            Style::new().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(mode_line), mode_area);

    // --- Content area (placeholder for now) ---
    let content = if let Some(file) = app.active_file() {
        let info = format!(
            "File: {}\nStatus: {:?}\nHunks: {}\nOld: {} bytes\nNew: {} bytes",
            file.path.display(),
            file.status,
            file.hunks.len(),
            file.old_content.len(),
            file.new_content.len(),
        );
        Paragraph::new(info).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Diff"),
        )
    } else {
        Paragraph::new("No file selected").block(
            Block::default()
                .borders(Borders::ALL)
                .title("Diff"),
        )
    };
    frame.render_widget(content, content_area);

    // --- Status bar ---
    let status = Line::from(vec![
        Span::styled(" [q]", Style::new().fg(Color::Yellow)),
        Span::raw("uit  "),
        Span::styled("[Tab]", Style::new().fg(Color::Yellow)),
        Span::raw(" next file  "),
        Span::styled("[s]", Style::new().fg(Color::Yellow)),
        Span::raw("taged  "),
        Span::styled("[w]", Style::new().fg(Color::Yellow)),
        Span::raw("orking tree  "),
        Span::styled("[n/N]", Style::new().fg(Color::Yellow)),
        Span::raw(" next/prev hunk  "),
        Span::styled("[c]", Style::new().fg(Color::Yellow)),
        Span::raw("ollapse"),
    ]);
    frame.render_widget(Paragraph::new(status), status_area);
}
```

**Step 2: Wire up the event loop in main.rs**

Replace `src/main.rs` entirely:

```rust
mod app;
mod diff;
mod ui;

use anyhow::Result;
use clap::Parser;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use std::path::PathBuf;
use std::time::Duration;

use app::App;
use diff::git2_provider::Git2Provider;
use diff::model::DiffMode;
use diff::provider::DiffProvider;

#[derive(Parser)]
#[command(name = "better-diff", about = "A better git diff viewer")]
struct Cli {
    /// Path to the git repository
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Start in staged mode
    #[arg(short, long)]
    staged: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut app = App::new(cli.path.clone());
    if cli.staged {
        app.mode = DiffMode::Staged;
    }

    // Compute initial diff
    let provider = Git2Provider::new();
    app.files = provider.compute_diff(&cli.path, app.mode)?;

    // Initialize terminal
    let mut terminal = ratatui::init();

    // Event loop
    let result = run_event_loop(&mut terminal, &mut app, &provider);

    // Restore terminal
    ratatui::restore();

    result
}

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    provider: &Git2Provider,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app.should_quit = true;
                    }
                    KeyCode::Tab => app.next_file(),
                    KeyCode::BackTab => app.prev_file(),
                    KeyCode::Char('j') | KeyCode::Down => app.scroll_down(),
                    KeyCode::Char('k') | KeyCode::Up => app.scroll_up(),
                    KeyCode::Char('n') => app.next_hunk(),
                    KeyCode::Char('N') => app.prev_hunk(),
                    KeyCode::Char('s') => {
                        app.mode = DiffMode::Staged;
                        app.files = provider.compute_diff(&app.repo_path, app.mode)?;
                        app.active_file = 0;
                        app.scroll_offset = 0;
                    }
                    KeyCode::Char('w') => {
                        app.mode = DiffMode::WorkingTree;
                        app.files = provider.compute_diff(&app.repo_path, app.mode)?;
                        app.active_file = 0;
                        app.scroll_offset = 0;
                    }
                    KeyCode::Char('c') => app.cycle_collapse(),
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        let idx = c.to_digit(10).unwrap() as usize;
                        if idx > 0 {
                            app.select_file(idx - 1);
                        }
                    }
                    _ => {}
                }

                if app.should_quit {
                    break;
                }
            }
        }
    }
    Ok(())
}
```

**Step 3: Run tests, then run the TUI**

Run: `cargo test`
Expected: All tests pass

Run: `cargo run`
Expected: TUI launches showing tab bar, mode indicator, placeholder content, status bar. Press `q` to quit.

**Step 4: Commit**

```bash
git add src/ui/ src/main.rs
git commit -m "feat: add TUI shell with event loop, tabs, and keybindings"
```

---

### Task 7: Side-by-Side Diff View

**Files:**
- Create: `src/ui/split_pane.rs`
- Modify: `src/ui/mod.rs`

**Step 1: Create the side-by-side diff renderer**

In `src/ui/split_pane.rs`:

```rust
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::diff::model::*;

/// Render a side-by-side diff view for the given file
pub fn render_split_pane(frame: &mut Frame, area: Rect, file: &FileDiff, scroll_offset: usize) {
    // Split into left and right panes
    let [left_area, right_area] = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ])
    .areas(area);

    let (old_lines, new_lines) = build_side_by_side_lines(file);

    let visible_height = left_area.height.saturating_sub(2) as usize; // -2 for border
    let max_scroll = old_lines.len().saturating_sub(visible_height);
    let scroll = scroll_offset.min(max_scroll);

    // Left pane (old)
    let old_visible: Vec<Line> = old_lines
        .into_iter()
        .skip(scroll)
        .take(visible_height)
        .collect();

    let left = Paragraph::new(old_visible).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("{} (old)", file.path.display())),
    );
    frame.render_widget(left, left_area);

    // Right pane (new)
    let new_visible: Vec<Line> = new_lines
        .into_iter()
        .skip(scroll)
        .take(visible_height)
        .collect();

    let right = Paragraph::new(new_visible).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("{} (new)", file.path.display())),
    );
    frame.render_widget(right, right_area);
}

/// Build left (old) and right (new) line lists from the hunks.
/// Modified lines get token-level highlighting.
fn build_side_by_side_lines<'a>(file: &'a FileDiff) -> (Vec<Line<'a>>, Vec<Line<'a>>) {
    let mut old_lines = Vec::new();
    let mut new_lines = Vec::new();

    for hunk in &file.hunks {
        // Hunk header
        let header = format!(
            "@@ -{},{} +{},{} @@",
            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
        );
        old_lines.push(Line::from(Span::styled(
            header.clone(),
            Style::new().fg(Color::DarkGray),
        )));
        new_lines.push(Line::from(Span::styled(
            header,
            Style::new().fg(Color::DarkGray),
        )));

        for line in &hunk.lines {
            match line.kind {
                LineKind::Context => {
                    let line_no_old = format_line_no(line.old_line_no);
                    let line_no_new = format_line_no(line.new_line_no);
                    let text = line.old_text.as_deref().unwrap_or("");

                    old_lines.push(Line::from(vec![
                        Span::styled(line_no_old, Style::new().fg(Color::DarkGray)),
                        Span::raw(text),
                    ]));
                    new_lines.push(Line::from(vec![
                        Span::styled(line_no_new, Style::new().fg(Color::DarkGray)),
                        Span::raw(text),
                    ]));
                }
                LineKind::Added => {
                    let line_no = format_line_no(line.new_line_no);
                    let text = line.new_text.as_deref().unwrap_or("");

                    old_lines.push(Line::from(""));
                    new_lines.push(Line::from(vec![
                        Span::styled(line_no, Style::new().fg(Color::DarkGray)),
                        Span::styled(
                            text.to_string(),
                            Style::new().fg(Color::Green).bg(Color::Rgb(0, 40, 0)),
                        ),
                    ]));
                }
                LineKind::Deleted => {
                    let line_no = format_line_no(line.old_line_no);
                    let text = line.old_text.as_deref().unwrap_or("");

                    old_lines.push(Line::from(vec![
                        Span::styled(line_no, Style::new().fg(Color::DarkGray)),
                        Span::styled(
                            text.to_string(),
                            Style::new().fg(Color::Red).bg(Color::Rgb(40, 0, 0)),
                        ),
                    ]));
                    new_lines.push(Line::from(""));
                }
                LineKind::Modified => {
                    let line_no_old = format_line_no(line.old_line_no);
                    let line_no_new = format_line_no(line.new_line_no);

                    if !line.tokens.is_empty() {
                        // Token-level highlighting
                        let mut old_spans = vec![
                            Span::styled(line_no_old, Style::new().fg(Color::DarkGray)),
                        ];
                        let mut new_spans = vec![
                            Span::styled(line_no_new, Style::new().fg(Color::DarkGray)),
                        ];

                        // Build old side from tokens that appear on old side
                        // (Equal, Deletion, Rename on old side)
                        for token in &line.tokens {
                            match token.kind {
                                ChangeKind::Equal => {
                                    old_spans.push(Span::styled(
                                        token.text.clone(),
                                        Style::new().bg(Color::Rgb(40, 0, 0)),
                                    ));
                                    new_spans.push(Span::styled(
                                        token.text.clone(),
                                        Style::new().bg(Color::Rgb(0, 40, 0)),
                                    ));
                                }
                                ChangeKind::Deletion => {
                                    old_spans.push(Span::styled(
                                        token.text.clone(),
                                        Style::new()
                                            .fg(Color::Red)
                                            .bg(Color::Rgb(80, 0, 0))
                                            .add_modifier(Modifier::BOLD),
                                    ));
                                }
                                ChangeKind::Addition => {
                                    new_spans.push(Span::styled(
                                        token.text.clone(),
                                        Style::new()
                                            .fg(Color::Green)
                                            .bg(Color::Rgb(0, 80, 0))
                                            .add_modifier(Modifier::BOLD),
                                    ));
                                }
                                ChangeKind::Rename => {
                                    old_spans.push(Span::styled(
                                        token.text.clone(),
                                        Style::new()
                                            .fg(Color::Blue)
                                            .bg(Color::Rgb(0, 0, 80))
                                            .add_modifier(Modifier::BOLD),
                                    ));
                                    new_spans.push(Span::styled(
                                        token.text.clone(),
                                        Style::new()
                                            .fg(Color::Blue)
                                            .bg(Color::Rgb(0, 0, 80))
                                            .add_modifier(Modifier::BOLD),
                                    ));
                                }
                            }
                        }

                        old_lines.push(Line::from(old_spans));
                        new_lines.push(Line::from(new_spans));
                    } else {
                        // Fallback: no token data, show as plain modified
                        let old_text = line.old_text.as_deref().unwrap_or("");
                        let new_text = line.new_text.as_deref().unwrap_or("");

                        old_lines.push(Line::from(vec![
                            Span::styled(line_no_old, Style::new().fg(Color::DarkGray)),
                            Span::styled(
                                old_text.to_string(),
                                Style::new().fg(Color::Red).bg(Color::Rgb(40, 0, 0)),
                            ),
                        ]));
                        new_lines.push(Line::from(vec![
                            Span::styled(line_no_new, Style::new().fg(Color::DarkGray)),
                            Span::styled(
                                new_text.to_string(),
                                Style::new().fg(Color::Green).bg(Color::Rgb(0, 40, 0)),
                            ),
                        ]));
                    }
                }
            }
        }
    }

    (old_lines, new_lines)
}

fn format_line_no(line_no: Option<usize>) -> String {
    match line_no {
        Some(n) => format!("{:>4} ", n),
        None => "     ".to_string(),
    }
}
```

**Step 2: Integrate split pane into main UI render**

In `src/ui/mod.rs`, replace the content area placeholder with:

```rust
pub mod split_pane;
```

And replace the content rendering block (the `let content = if let Some(file)...` section) with:

```rust
    // --- Content area ---
    if let Some(file) = app.active_file() {
        split_pane::render_split_pane(frame, content_area, file, app.scroll_offset);
    } else {
        let empty = Paragraph::new("No changes to display").block(
            Block::default()
                .borders(Borders::ALL)
                .title("Diff"),
        );
        frame.render_widget(empty, content_area);
    }
```

Remove the now-unused `Borders` import from `Block` usage at top if needed (keep it if `split_pane` uses it).

**Step 3: Run and verify**

Run: `cargo test && cargo run`
Expected: TUI shows side-by-side diff with line numbers, token-level color highlighting on modified lines. Scrolling with j/k works. Tab switches files.

**Step 4: Commit**

```bash
git add src/ui/
git commit -m "feat: add side-by-side diff view with token-level highlighting"
```

---

### Task 8: File Watcher Integration

**Files:**
- Create: `src/watcher.rs`
- Modify: `src/main.rs`

**Step 1: Create watcher module**

In `src/watcher.rs`:

```rust
use anyhow::Result;
use crossbeam_channel::Sender;
use notify::{RecursiveMode, Watcher};
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::path::Path;
use std::time::Duration;

/// Events sent from the watcher to the main event loop
#[derive(Debug)]
pub enum WatchEvent {
    /// Files changed on disk, re-diff needed
    FilesChanged,
}

/// Start watching a directory for changes.
/// Sends WatchEvent::FilesChanged through the channel when files change.
/// Returns the debouncer (must be kept alive for watching to continue).
pub fn start_watching(
    path: &Path,
    sender: Sender<WatchEvent>,
) -> Result<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>> {
    let mut debouncer = new_debouncer(
        Duration::from_millis(50),
        move |events: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            if let Ok(events) = events {
                let has_relevant = events.iter().any(|e| {
                    // Ignore .git directory changes
                    let is_git = e
                        .path
                        .components()
                        .any(|c| c.as_os_str() == ".git");
                    !is_git && e.kind == DebouncedEventKind::Any
                });
                if has_relevant {
                    let _ = sender.send(WatchEvent::FilesChanged);
                }
            }
        },
    )?;

    debouncer.watcher().watch(path, RecursiveMode::Recursive)?;

    Ok(debouncer)
}
```

**Step 2: Integrate watcher into main event loop**

In `src/main.rs`, update to use crossbeam-channel select:

```rust
mod app;
mod diff;
mod ui;
mod watcher;

use anyhow::Result;
use clap::Parser;
use crossbeam_channel::{select, unbounded};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use std::path::PathBuf;
use std::time::Duration;

use app::App;
use diff::git2_provider::Git2Provider;
use diff::model::DiffMode;
use diff::provider::DiffProvider;

#[derive(Parser)]
#[command(name = "better-diff", about = "A better git diff viewer")]
struct Cli {
    /// Path to the git repository
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Start in staged mode
    #[arg(short, long)]
    staged: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut app = App::new(cli.path.clone());
    if cli.staged {
        app.mode = DiffMode::Staged;
    }

    let provider = Git2Provider::new();
    app.files = provider.compute_diff(&cli.path, app.mode)?;

    // Start file watcher
    let (watch_tx, watch_rx) = unbounded();
    let _watcher = watcher::start_watching(&cli.path, watch_tx)?;

    let mut terminal = ratatui::init();
    let result = run_event_loop(&mut terminal, &mut app, &provider, &watch_rx);
    ratatui::restore();

    result
}

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    provider: &Git2Provider,
    watch_rx: &crossbeam_channel::Receiver<watcher::WatchEvent>,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        // Check for file watcher events (non-blocking)
        while let Ok(watcher::WatchEvent::FilesChanged) = watch_rx.try_recv() {
            let prev_file = app.active_file().map(|f| f.path.clone());
            app.files = provider.compute_diff(&app.repo_path, app.mode)?;

            // Try to keep the same file selected
            if let Some(prev) = &prev_file {
                if let Some(idx) = app.files.iter().position(|f| &f.path == prev) {
                    app.active_file = idx;
                } else {
                    app.active_file = 0;
                    app.scroll_offset = 0;
                }
            }
        }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app.should_quit = true;
                    }
                    KeyCode::Tab => app.next_file(),
                    KeyCode::BackTab => app.prev_file(),
                    KeyCode::Char('j') | KeyCode::Down => app.scroll_down(),
                    KeyCode::Char('k') | KeyCode::Up => app.scroll_up(),
                    KeyCode::Char('n') => app.next_hunk(),
                    KeyCode::Char('N') => app.prev_hunk(),
                    KeyCode::Char('s') => {
                        app.mode = DiffMode::Staged;
                        app.files = provider.compute_diff(&app.repo_path, app.mode)?;
                        app.active_file = 0;
                        app.scroll_offset = 0;
                    }
                    KeyCode::Char('w') => {
                        app.mode = DiffMode::WorkingTree;
                        app.files = provider.compute_diff(&app.repo_path, app.mode)?;
                        app.active_file = 0;
                        app.scroll_offset = 0;
                    }
                    KeyCode::Char('c') => app.cycle_collapse(),
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        let idx = c.to_digit(10).unwrap() as usize;
                        if idx > 0 {
                            app.select_file(idx - 1);
                        }
                    }
                    _ => {}
                }

                if app.should_quit {
                    break;
                }
            }
        }
    }
    Ok(())
}
```

**Step 3: Run and verify**

Run: `cargo test && cargo run`
Expected: TUI launches. Edit a file in another terminal — the diff view updates automatically within ~100ms.

**Step 4: Commit**

```bash
git add src/watcher.rs src/main.rs
git commit -m "feat: add filesystem watcher for real-time diff updates"
```

---

### Task 9: Tree-Sitter Syntax Highlighting

**Files:**
- Create: `src/syntax.rs`
- Modify: `src/ui/split_pane.rs`

**Step 1: Create syntax highlighting module**

In `src/syntax.rs`:

```rust
use ratatui::style::{Color, Style};
use tree_sitter::{Parser, Query, QueryCursor};

/// Syntax highlight spans for a line of code
#[derive(Debug, Clone)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

/// Parse source code and return highlight spans per line.
/// Returns a Vec indexed by line number (0-based), each containing spans for that line.
pub fn highlight_rust(source: &str) -> Vec<Vec<HighlightSpan>> {
    let mut parser = Parser::new();
    let language = tree_sitter_rust::LANGUAGE;
    parser
        .set_language(&language.into())
        .expect("Failed to set language");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![vec![]; source.lines().count()],
    };

    // Basic highlight query for Rust
    let query_source = r#"
        (line_comment) @comment
        (block_comment) @comment
        (string_literal) @string
        (raw_string_literal) @string
        (char_literal) @string
        (integer_literal) @number
        (float_literal) @number
        (boolean_literal) @boolean
        (type_identifier) @type
        (primitive_type) @type
        (self) @keyword
        ["fn" "let" "mut" "pub" "use" "mod" "struct" "enum" "impl"
         "trait" "for" "while" "loop" "if" "else" "match" "return"
         "async" "await" "move" "ref" "where" "type" "const" "static"
         "unsafe" "extern" "crate" "super" "as" "in" "break" "continue"
         "dyn"] @keyword
        (function_item name: (identifier) @function)
        (call_expression function: (identifier) @function_call)
        (macro_invocation macro: (identifier) @macro)
    "#;

    let query = match Query::new(&language.into(), query_source) {
        Ok(q) => q,
        Err(_) => return vec![vec![]; source.lines().count()],
    };

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    let line_count = source.lines().count().max(1);
    let mut highlights: Vec<Vec<HighlightSpan>> = vec![vec![]; line_count];

    // Build line offset table
    let line_offsets: Vec<usize> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    for m in matches {
        for capture in m.captures {
            let node = capture.node;
            let capture_name = &query.capture_names()[capture.index as usize];
            let style = capture_to_style(capture_name);

            let start_byte = node.start_byte();
            let end_byte = node.end_byte();
            let start_line = node.start_position().row;
            let end_line = node.end_position().row;

            for line in start_line..=end_line {
                if line >= line_count {
                    break;
                }

                let line_start = line_offsets.get(line).copied().unwrap_or(0);
                let span_start = if line == start_line {
                    start_byte - line_start
                } else {
                    0
                };
                let span_end = if line == end_line {
                    end_byte - line_start
                } else {
                    source[line_start..]
                        .find('\n')
                        .unwrap_or(source.len() - line_start)
                };

                highlights[line].push(HighlightSpan {
                    start: span_start,
                    end: span_end,
                    style,
                });
            }
        }
    }

    highlights
}

fn capture_to_style(name: &str) -> Style {
    match name {
        "comment" => Style::new().fg(Color::DarkGray),
        "string" => Style::new().fg(Color::Rgb(206, 145, 120)),
        "number" | "boolean" => Style::new().fg(Color::Rgb(181, 206, 168)),
        "type" => Style::new().fg(Color::Rgb(78, 201, 176)),
        "keyword" => Style::new().fg(Color::Rgb(197, 134, 192)),
        "function" | "function_call" => Style::new().fg(Color::Rgb(220, 220, 170)),
        "macro" => Style::new().fg(Color::Rgb(86, 156, 214)),
        _ => Style::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_rust_basic() {
        let source = "fn main() {\n    let x = 42;\n}\n";
        let highlights = highlight_rust(source);

        // Should have highlights for at least the first line (fn keyword, main function name)
        assert!(!highlights.is_empty());
        assert!(!highlights[0].is_empty()); // "fn" and "main" should be highlighted
    }

    #[test]
    fn test_highlight_empty_source() {
        let highlights = highlight_rust("");
        assert_eq!(highlights.len(), 1); // at least one empty line
    }

    #[test]
    fn test_highlight_preserves_line_count() {
        let source = "line1\nline2\nline3\n";
        let highlights = highlight_rust(source);
        assert_eq!(highlights.len(), 3);
    }
}
```

**Step 2: Run tests**

Run: `cargo test`
Expected: All tests pass

**Step 3: Commit**

Note: Integrating syntax highlighting into the split pane view is deferred to after the side-by-side view is working well. The `highlight_rust` function is ready to be called from the UI layer when rendering context lines.

```bash
git add src/syntax.rs
git commit -m "feat: add tree-sitter syntax highlighting for Rust"
```

---

### Task 10: Structural Folding

**Files:**
- Create: `src/diff/folding.rs`
- Modify: `src/diff/mod.rs`

**Step 1: Implement AST-based fold region detection**

In `src/diff/folding.rs`:

```rust
use tree_sitter::Parser;

use super::model::{FoldKind, FoldRegion};

/// Compute fold regions from source code using tree-sitter.
/// Returns regions that represent functions, impls, structs, etc.
pub fn compute_fold_regions(source: &str) -> Vec<FoldRegion> {
    let mut parser = Parser::new();
    let language = tree_sitter_rust::LANGUAGE;
    parser
        .set_language(&language.into())
        .expect("Failed to set language");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };

    let mut regions = Vec::new();
    collect_fold_nodes(tree.root_node(), source, &mut regions);
    regions
}

fn collect_fold_nodes(
    node: tree_sitter::Node,
    source: &str,
    regions: &mut Vec<FoldRegion>,
) {
    let kind = node.kind();
    let fold_kind = match kind {
        "function_item" => Some(FoldKind::Function),
        "impl_item" => Some(FoldKind::Impl),
        "mod_item" => Some(FoldKind::Module),
        "struct_item" => Some(FoldKind::Struct),
        "enum_item" => Some(FoldKind::Enum),
        _ => None,
    };

    if let Some(fk) = fold_kind {
        let start_line = node.start_position().row + 1; // 1-indexed
        let end_line = node.end_position().row + 1;

        // Only fold regions spanning more than 3 lines
        if end_line - start_line > 3 {
            let label = build_fold_label(node, source, kind);
            regions.push(FoldRegion {
                kind: fk,
                label,
                old_start: start_line,
                old_end: end_line,
                new_start: start_line,
                new_end: end_line,
                is_collapsed: true,
            });
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_fold_nodes(child, source, regions);
    }
}

fn build_fold_label(node: tree_sitter::Node, source: &str, kind: &str) -> String {
    // Try to extract the name from the node
    let name = node
        .child_by_field_name("name")
        .map(|n| &source[n.byte_range()])
        .unwrap_or("...");

    let start = node.start_position().row + 1;
    let end = node.end_position().row + 1;
    let line_count = end - start + 1;

    match kind {
        "function_item" => format!("fn {}() ({} lines)", name, line_count),
        "impl_item" => format!("impl {} ({} lines)", name, line_count),
        "mod_item" => format!("mod {} ({} lines)", name, line_count),
        "struct_item" => format!("struct {} ({} lines)", name, line_count),
        "enum_item" => format!("enum {} ({} lines)", name, line_count),
        _ => format!("{} ({} lines)", name, line_count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fold_function() {
        let source = r#"
fn hello() {
    let a = 1;
    let b = 2;
    let c = 3;
    let d = 4;
}
"#;
        let regions = compute_fold_regions(source);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].kind, FoldKind::Function);
        assert!(regions[0].label.contains("hello"));
    }

    #[test]
    fn test_fold_impl() {
        let source = r#"
struct Foo;

impl Foo {
    fn bar(&self) {
        todo!()
    }

    fn baz(&self) {
        todo!()
    }
}
"#;
        let regions = compute_fold_regions(source);
        let impl_regions: Vec<_> = regions.iter().filter(|r| r.kind == FoldKind::Impl).collect();
        assert!(!impl_regions.is_empty());
        assert!(impl_regions[0].label.contains("Foo"));
    }

    #[test]
    fn test_short_function_not_folded() {
        let source = r#"
fn tiny() {
    1
}
"#;
        let regions = compute_fold_regions(source);
        // 3 lines or fewer should not be folded
        assert!(regions.is_empty());
    }

    #[test]
    fn test_empty_source() {
        let regions = compute_fold_regions("");
        assert!(regions.is_empty());
    }
}
```

**Step 2: Update diff/mod.rs**

```rust
pub mod model;
pub mod provider;
pub mod git2_provider;
pub mod tokens;
pub mod folding;
```

**Step 3: Integrate folding into git2_provider**

In `src/diff/git2_provider.rs`, after loading file contents, add fold region computation:

```rust
use super::folding::compute_fold_regions;

// After loading file contents:
for file_diff in &mut file_diffs {
    if !file_diff.new_content.is_empty() {
        file_diff.fold_regions = compute_fold_regions(&file_diff.new_content);
    } else if !file_diff.old_content.is_empty() {
        file_diff.fold_regions = compute_fold_regions(&file_diff.old_content);
    }
}
```

**Step 4: Run tests**

Run: `cargo test`
Expected: All tests pass

**Step 5: Commit**

```bash
git add src/diff/folding.rs src/diff/mod.rs src/diff/git2_provider.rs
git commit -m "feat: add AST-based structural folding via tree-sitter"
```

---

### Task 11: Block Move Detection

**Files:**
- Create: `src/diff/moves.rs`
- Modify: `src/diff/mod.rs`

**Step 1: Implement move detection**

In `src/diff/moves.rs`:

```rust
use std::collections::HashMap;
use std::path::PathBuf;

use super::model::{FileDiff, LineKind, MoveMatch};

const MIN_BLOCK_SIZE: usize = 3;
const SIMILARITY_THRESHOLD: f64 = 0.8;

/// Detect moved blocks within and across files.
pub fn detect_moves(files: &mut Vec<FileDiff>) {
    // Collect all deleted and added blocks across all files
    let mut deleted_blocks: Vec<(PathBuf, usize, Vec<String>)> = Vec::new();
    let mut added_blocks: Vec<(PathBuf, usize, Vec<String>)> = Vec::new();

    for file in files.iter() {
        for hunk in &file.hunks {
            let mut current_deleted: Vec<String> = Vec::new();
            let mut del_start = 0;
            let mut current_added: Vec<String> = Vec::new();
            let mut add_start = 0;

            for line in &hunk.lines {
                match line.kind {
                    LineKind::Deleted => {
                        if current_deleted.is_empty() {
                            del_start = line.old_line_no.unwrap_or(0);
                        }
                        current_deleted.push(normalize(
                            line.old_text.as_deref().unwrap_or(""),
                        ));

                        // Flush added
                        if current_added.len() >= MIN_BLOCK_SIZE {
                            added_blocks.push((
                                file.path.clone(),
                                add_start,
                                std::mem::take(&mut current_added),
                            ));
                        } else {
                            current_added.clear();
                        }
                    }
                    LineKind::Added => {
                        if current_added.is_empty() {
                            add_start = line.new_line_no.unwrap_or(0);
                        }
                        current_added.push(normalize(
                            line.new_text.as_deref().unwrap_or(""),
                        ));

                        // Flush deleted
                        if current_deleted.len() >= MIN_BLOCK_SIZE {
                            deleted_blocks.push((
                                file.path.clone(),
                                del_start,
                                std::mem::take(&mut current_deleted),
                            ));
                        } else {
                            current_deleted.clear();
                        }
                    }
                    _ => {
                        // Flush both
                        if current_deleted.len() >= MIN_BLOCK_SIZE {
                            deleted_blocks.push((
                                file.path.clone(),
                                del_start,
                                std::mem::take(&mut current_deleted),
                            ));
                        } else {
                            current_deleted.clear();
                        }
                        if current_added.len() >= MIN_BLOCK_SIZE {
                            added_blocks.push((
                                file.path.clone(),
                                add_start,
                                std::mem::take(&mut current_added),
                            ));
                        } else {
                            current_added.clear();
                        }
                    }
                }
            }

            // Flush remaining
            if current_deleted.len() >= MIN_BLOCK_SIZE {
                deleted_blocks.push((file.path.clone(), del_start, current_deleted));
            }
            if current_added.len() >= MIN_BLOCK_SIZE {
                added_blocks.push((file.path.clone(), add_start, current_added));
            }
        }
    }

    // Match deleted blocks with added blocks
    let mut matches: Vec<MoveMatch> = Vec::new();
    let mut used_adds: Vec<bool> = vec![false; added_blocks.len()];

    for (del_path, del_start, del_lines) in &deleted_blocks {
        let del_hash: Vec<u64> = del_lines.iter().map(|l| hash_line(l)).collect();

        let mut best_match: Option<(usize, f64)> = None;

        for (add_idx, (add_path, _add_start, add_lines)) in added_blocks.iter().enumerate() {
            if used_adds[add_idx] {
                continue;
            }

            let sim = block_similarity(&del_hash, add_lines);
            if sim >= SIMILARITY_THRESHOLD {
                if best_match.is_none() || sim > best_match.unwrap().1 {
                    best_match = Some((add_idx, sim));
                }
            }
        }

        if let Some((add_idx, sim)) = best_match {
            used_adds[add_idx] = true;
            let (add_path, add_start, add_lines) = &added_blocks[add_idx];
            matches.push(MoveMatch {
                source_file: del_path.clone(),
                source_start: *del_start,
                source_end: del_start + del_lines.len(),
                dest_file: add_path.clone(),
                dest_start: *add_start,
                dest_end: add_start + add_lines.len(),
                similarity: sim,
            });
        }
    }

    // Distribute matches back to files
    for file in files.iter_mut() {
        file.move_matches = matches
            .iter()
            .filter(|m| m.source_file == file.path || m.dest_file == file.path)
            .cloned()
            .collect();
    }
}

/// Normalize a line for comparison (trim whitespace)
fn normalize(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Simple hash for a normalized line
fn hash_line(line: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

/// Compute similarity between a hashed block and a raw block
fn block_similarity(hashed: &[u64], other_lines: &[String]) -> f64 {
    let other_hashed: Vec<u64> = other_lines.iter().map(|l| hash_line(l)).collect();
    let len = hashed.len().max(other_hashed.len());
    if len == 0 {
        return 0.0;
    }

    let matching = hashed
        .iter()
        .zip(other_hashed.iter())
        .filter(|(a, b)| a == b)
        .count();

    matching as f64 / len as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::model::*;

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("  let   x = 1;  "), "let x = 1;");
    }

    #[test]
    fn test_identical_blocks_match() {
        let lines = vec!["let a = 1;", "let b = 2;", "let c = 3;", "let d = 4;"];
        let hashed: Vec<u64> = lines.iter().map(|l| hash_line(l)).collect();
        let other: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        assert_eq!(block_similarity(&hashed, &other), 1.0);
    }

    #[test]
    fn test_detect_move_within_file() {
        let mut files = vec![FileDiff {
            path: PathBuf::from("test.rs"),
            status: FileStatus::Modified,
            hunks: vec![
                Hunk {
                    old_start: 10,
                    new_start: 10,
                    old_lines: 4,
                    new_lines: 0,
                    lines: vec![
                        DiffLine {
                            kind: LineKind::Deleted,
                            old_line_no: Some(10),
                            new_line_no: None,
                            old_text: Some("fn helper() {".into()),
                            new_text: None,
                            tokens: vec![],
                        },
                        DiffLine {
                            kind: LineKind::Deleted,
                            old_line_no: Some(11),
                            new_line_no: None,
                            old_text: Some("    do_thing_a();".into()),
                            new_text: None,
                            tokens: vec![],
                        },
                        DiffLine {
                            kind: LineKind::Deleted,
                            old_line_no: Some(12),
                            new_line_no: None,
                            old_text: Some("    do_thing_b();".into()),
                            new_text: None,
                            tokens: vec![],
                        },
                        DiffLine {
                            kind: LineKind::Deleted,
                            old_line_no: Some(13),
                            new_line_no: None,
                            old_text: Some("}".into()),
                            new_text: None,
                            tokens: vec![],
                        },
                    ],
                },
                Hunk {
                    old_start: 50,
                    new_start: 46,
                    old_lines: 0,
                    new_lines: 4,
                    lines: vec![
                        DiffLine {
                            kind: LineKind::Added,
                            old_line_no: None,
                            new_line_no: Some(46),
                            old_text: None,
                            new_text: Some("fn helper() {".into()),
                            tokens: vec![],
                        },
                        DiffLine {
                            kind: LineKind::Added,
                            old_line_no: None,
                            new_line_no: Some(47),
                            old_text: None,
                            new_text: Some("    do_thing_a();".into()),
                            tokens: vec![],
                        },
                        DiffLine {
                            kind: LineKind::Added,
                            old_line_no: None,
                            new_line_no: Some(48),
                            old_text: None,
                            new_text: Some("    do_thing_b();".into()),
                            tokens: vec![],
                        },
                        DiffLine {
                            kind: LineKind::Added,
                            old_line_no: None,
                            new_line_no: Some(49),
                            old_text: None,
                            new_text: Some("}".into()),
                            tokens: vec![],
                        },
                    ],
                },
            ],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }];

        detect_moves(&mut files);
        assert!(!files[0].move_matches.is_empty());
        assert!(files[0].move_matches[0].similarity >= 0.8);
    }
}
```

**Step 2: Update diff/mod.rs**

```rust
pub mod model;
pub mod provider;
pub mod git2_provider;
pub mod tokens;
pub mod folding;
pub mod moves;
```

**Step 3: Run tests**

Run: `cargo test`
Expected: All tests pass

**Step 4: Commit**

```bash
git add src/diff/moves.rs src/diff/mod.rs
git commit -m "feat: add block move detection with hash-based matching"
```

---

### Task 12: Heat Map Minimap Widget

**Files:**
- Create: `src/ui/minimap.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/split_pane.rs`

**Step 1: Create minimap widget**

In `src/ui/minimap.rs`:

```rust
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::buffer::Buffer;
use ratatui::widgets::Widget;

use crate::diff::model::*;

pub struct Minimap<'a> {
    file: &'a FileDiff,
    scroll_offset: usize,
    visible_height: usize,
}

impl<'a> Minimap<'a> {
    pub fn new(file: &'a FileDiff, scroll_offset: usize, visible_height: usize) -> Self {
        Self {
            file,
            scroll_offset,
            visible_height,
        }
    }
}

impl<'a> Widget for Minimap<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Count total lines and build a density map
        let total_lines: usize = self.file.hunks.iter().map(|h| h.lines.len()).sum();
        if total_lines == 0 {
            return;
        }

        let height = area.height as usize;

        // Build change density per minimap row
        let mut densities = vec![0.0f64; height];
        let mut line_idx = 0;

        for hunk in &self.file.hunks {
            for line in &hunk.lines {
                let map_row = (line_idx * height) / total_lines.max(1);
                let map_row = map_row.min(height - 1);

                match line.kind {
                    LineKind::Added | LineKind::Modified => densities[map_row] += 1.0,
                    LineKind::Deleted => densities[map_row] += 0.8,
                    LineKind::Context => densities[map_row] += 0.0,
                }

                line_idx += 1;
            }
        }

        // Normalize densities
        let max_density = densities.iter().cloned().fold(0.0, f64::max);
        if max_density > 0.0 {
            for d in &mut densities {
                *d /= max_density;
            }
        }

        // Render
        for (row, &density) in densities.iter().enumerate() {
            let y = area.y + row as u16;
            if y >= area.y + area.height {
                break;
            }

            let color = if density > 0.7 {
                Color::Rgb(255, 100, 50)  // Hot
            } else if density > 0.3 {
                Color::Rgb(200, 150, 50)  // Warm
            } else if density > 0.0 {
                Color::Rgb(80, 80, 40)    // Cool
            } else {
                Color::Rgb(30, 30, 30)    // None
            };

            // Draw the minimap column
            for x in area.x..area.x + area.width {
                buf.set_string(
                    x,
                    y,
                    if density > 0.0 { "█" } else { "▕" },
                    Style::new().fg(color),
                );
            }
        }

        // Draw scroll position indicator
        if total_lines > 0 && self.visible_height > 0 {
            let scroll_row =
                (self.scroll_offset * height) / total_lines.max(1);
            let scroll_height =
                ((self.visible_height * height) / total_lines.max(1)).max(1);

            for row in scroll_row..=(scroll_row + scroll_height).min(height - 1) {
                let y = area.y + row as u16;
                if y < area.y + area.height {
                    for x in area.x..area.x + area.width {
                        buf.set_style(
                            x,
                            y,
                            Style::new().bg(Color::Rgb(60, 60, 80)),
                        );
                    }
                }
            }
        }
    }
}
```

**Step 2: Integrate minimap into split pane layout**

In `src/ui/split_pane.rs`, modify the layout to add a minimap column on the right:

Change the horizontal split from:

```rust
let [left_area, right_area] = Layout::horizontal([
    Constraint::Percentage(50),
    Constraint::Percentage(50),
])
.areas(area);
```

To:

```rust
let [left_area, right_area, minimap_area] = Layout::horizontal([
    Constraint::Percentage(49),
    Constraint::Percentage(49),
    Constraint::Length(2),
])
.areas(area);

// Render minimap
use super::minimap::Minimap;
let visible_height = left_area.height.saturating_sub(2) as usize;
frame.render_widget(
    Minimap::new(file, scroll_offset, visible_height),
    minimap_area,
);
```

**Step 3: Update ui/mod.rs**

Add: `pub mod minimap;`

**Step 4: Run and verify**

Run: `cargo test && cargo run`
Expected: Minimap renders on the right edge showing change density as colored blocks.

**Step 5: Commit**

```bash
git add src/ui/minimap.rs src/ui/split_pane.rs src/ui/mod.rs
git commit -m "feat: add heat map minimap sidebar for change density"
```

---

### Task 13: Change Animation System

**Files:**
- Create: `src/ui/animation.rs`
- Modify: `src/app.rs`
- Modify: `src/main.rs`

**Step 1: Create animation state and logic**

In `src/ui/animation.rs`:

```rust
use std::time::{Duration, Instant};

/// Tracks animation state for hunk focus transitions
#[derive(Debug, Clone)]
pub struct AnimationState {
    /// When the animation started
    pub started_at: Instant,
    /// Total duration of the animation
    pub duration: Duration,
    /// Which hunk index is being animated
    pub hunk_index: usize,
}

impl AnimationState {
    pub fn new(hunk_index: usize) -> Self {
        Self {
            started_at: Instant::now(),
            duration: Duration::from_millis(150),
            hunk_index,
        }
    }

    /// Returns animation progress from 0.0 to 1.0
    pub fn progress(&self) -> f64 {
        let elapsed = self.started_at.elapsed();
        (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0)
    }

    /// Returns true if the animation is complete
    pub fn is_done(&self) -> bool {
        self.started_at.elapsed() >= self.duration
    }

    /// Returns the opacity for "fading in" elements (0.0 = invisible, 1.0 = full)
    pub fn fade_in_opacity(&self) -> f64 {
        self.progress()
    }

    /// Returns the opacity for "fading out" elements (1.0 = full, 0.0 = invisible)
    pub fn fade_out_opacity(&self) -> f64 {
        1.0 - self.progress()
    }

    /// Map opacity (0.0-1.0) to a terminal-friendly brightness value (0-255)
    pub fn opacity_to_brightness(opacity: f64) -> u8 {
        (opacity * 255.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_animation_progress() {
        let anim = AnimationState::new(0);
        assert!(anim.progress() < 0.5);
        assert!(!anim.is_done());
    }

    #[test]
    fn test_animation_completes() {
        let mut anim = AnimationState::new(0);
        anim.duration = Duration::from_millis(10);
        sleep(Duration::from_millis(20));
        assert!(anim.is_done());
        assert!((anim.progress() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_opacity_to_brightness() {
        assert_eq!(AnimationState::opacity_to_brightness(0.0), 0);
        assert_eq!(AnimationState::opacity_to_brightness(1.0), 255);
        assert_eq!(AnimationState::opacity_to_brightness(0.5), 127);
    }
}
```

**Step 2: Add animation state to App**

In `src/app.rs`, add the field and update `next_hunk`/`prev_hunk` to trigger animations:

Add field to `App`:

```rust
pub animation: Option<crate::ui::animation::AnimationState>,
```

Initialize in `App::new`:

```rust
animation: None,
```

Update `next_hunk` and `prev_hunk` to set animation state when jumping:

At the end of `next_hunk`, after setting `self.scroll_offset`:

```rust
self.animation = Some(crate::ui::animation::AnimationState::new(0));
```

Same for `prev_hunk`.

**Step 3: Add tick handling to event loop**

In the event loop in `src/main.rs`, after handling key events, add animation tick cleanup:

```rust
// Clear completed animations
if let Some(ref anim) = app.animation {
    if anim.is_done() {
        app.animation = None;
    }
}
```

**Step 4: Update ui/mod.rs**

Add: `pub mod animation;`

**Step 5: Run tests**

Run: `cargo test`
Expected: All tests pass

**Step 6: Commit**

```bash
git add src/ui/animation.rs src/app.rs src/main.rs src/ui/mod.rs
git commit -m "feat: add change animation system for hunk focus transitions"
```

---

### Task 14: Polish & Integration Testing

**Files:**
- Modify: various files for polish
- Create: `tests/integration.rs`

**Step 1: Create integration test**

In `tests/integration.rs`:

```rust
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn setup_repo_with_changes() -> TempDir {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    Command::new("git").args(["init"]).current_dir(path).output().unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(path)
        .output()
        .unwrap();

    // Create initial Rust file
    std::fs::write(
        path.join("main.rs"),
        r#"fn main() {
    let x = foo(a, b);
    println!("{}", x);
}

fn helper() {
    do_thing_a();
    do_thing_b();
    do_thing_c();
    do_thing_d();
}
"#,
    )
    .unwrap();

    Command::new("git").args(["add", "."]).current_dir(path).output().unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(path)
        .output()
        .unwrap();

    // Make changes
    std::fs::write(
        path.join("main.rs"),
        r#"fn main() {
    let x = bar(a, b, c);
    println!("{}", x);
}

fn setup() {
    init_config();
    init_logging();
    init_database();
    init_server();
}

fn helper() {
    do_thing_a();
    do_thing_b();
    do_thing_c();
    do_thing_d();
}
"#,
    )
    .unwrap();

    dir
}

#[test]
fn test_full_diff_pipeline() {
    use better_diff::diff::git2_provider::Git2Provider;
    use better_diff::diff::model::*;
    use better_diff::diff::provider::DiffProvider;

    let dir = setup_repo_with_changes();
    let provider = Git2Provider::new();
    let files = provider
        .compute_diff(dir.path(), DiffMode::WorkingTree)
        .unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, FileStatus::Modified);
    assert!(!files[0].hunks.is_empty());

    // Check that token-level diffs were computed
    let has_tokens = files[0]
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .any(|l| !l.tokens.is_empty());
    assert!(has_tokens, "Should have token-level diffs for modified lines");

    // Check that fold regions were computed
    assert!(
        !files[0].fold_regions.is_empty(),
        "Should detect foldable regions in Rust code"
    );
}
```

**Step 2: Make modules public in lib.rs**

Create `src/lib.rs`:

```rust
pub mod app;
pub mod diff;
pub mod syntax;
pub mod ui;
pub mod watcher;
```

**Step 3: Run all tests**

Run: `cargo test`
Expected: All unit tests and integration test pass

**Step 4: Commit**

```bash
git add tests/ src/lib.rs
git commit -m "feat: add integration tests and expose public API"
```

---

## Summary

| Task | What it builds | Key crates |
|---|---|---|
| 1 | Project scaffolding | all deps |
| 2 | Data model types | — |
| 3 | Git2 line-level diffing | git2 |
| 4 | Word-level token diffing | similar |
| 5 | App state & CLI | clap |
| 6 | TUI shell & event loop | ratatui, crossterm |
| 7 | Side-by-side diff view | ratatui |
| 8 | File watcher | notify |
| 9 | Syntax highlighting | tree-sitter |
| 10 | Structural folding | tree-sitter |
| 11 | Block move detection | — |
| 12 | Heat map minimap | ratatui |
| 13 | Change animations | ratatui |
| 14 | Integration tests & polish | — |

Each task builds on the previous and is independently testable and committable.
