use std::path::PathBuf;

use anyhow::Result;
use git2::Repository;

use crate::diff::git2_provider::Git2Provider;
use crate::diff::model::{CollapseLevel, DiffMode, FileDiff};
use crate::diff::provider::DiffProvider;
use crate::syntax::HighlightSpan;
use crate::ui::split_pane::RenderedFileLayout;
use crate::worktree::WorktreeManager;

/// Cached syntax highlights for a single file, keyed by file index.
#[derive(Default)]
pub struct RenderCache {
    /// The file index these highlights were computed for.
    pub cached_file_index: Option<usize>,
    pub old_highlights: Vec<Vec<HighlightSpan>>,
    pub new_highlights: Vec<Vec<HighlightSpan>>,

    /// Cached rendered split-pane layout for the active file/collapse level.
    pub cached_layout_file_index: Option<usize>,
    pub cached_layout_collapse_level: Option<CollapseLevel>,
    pub layout: RenderedFileLayout,
    pub layout_rebuild_count: usize,
}

impl RenderCache {
    pub fn new() -> Self {
        Self {
            cached_file_index: None,
            old_highlights: Vec::new(),
            new_highlights: Vec::new(),
            cached_layout_file_index: None,
            cached_layout_collapse_level: None,
            layout: RenderedFileLayout::default(),
            layout_rebuild_count: 0,
        }
    }

    /// Invalidate the cache so highlights are recomputed on next render.
    pub fn invalidate(&mut self) {
        self.cached_file_index = None;
        self.old_highlights.clear();
        self.new_highlights.clear();

        self.cached_layout_file_index = None;
        self.cached_layout_collapse_level = None;
        self.layout = RenderedFileLayout::default();
        self.layout_rebuild_count = 0;
    }

    pub fn ensure_layout(
        &mut self,
        file_index: usize,
        file: &FileDiff,
        collapse_level: CollapseLevel,
    ) -> &RenderedFileLayout {
        if self.cached_layout_file_index != Some(file_index)
            || self.cached_layout_collapse_level != Some(collapse_level)
            || self.cached_file_index != Some(file_index)
        {
            self.layout = crate::ui::split_pane::build_rendered_file_layout(
                file,
                &self.old_highlights,
                &self.new_highlights,
                collapse_level,
            );
            self.layout_rebuild_count = self.layout_rebuild_count.saturating_add(1);
            self.cached_layout_file_index = Some(file_index);
            self.cached_layout_collapse_level = Some(collapse_level);
        }

        &self.layout
    }
}

/// Resolve the branch label from an open repository.
pub fn resolve_branch_label(repo: &Repository) -> String {
    let head = match repo.head() {
        Ok(head) => head,
        Err(_) => return String::from("unknown"),
    };

    if head.is_branch() {
        return head.shorthand().unwrap_or("unknown").to_string();
    }

    let target = match head.target() {
        Some(oid) => oid,
        None => return String::from("unknown"),
    };

    let sha = target.to_string();
    format!("detached@{}", &sha[..8])
}

/// Per-worktree state: everything that is specific to a single worktree.
pub struct WorktreeContext {
    pub repo_path: PathBuf,
    pub branch_label: String,
    pub mode: DiffMode,
    pub files: Vec<FileDiff>,
    pub active_file: usize,
    pub scroll_offset: usize,
    pub collapse_level: CollapseLevel,
    pub animation: Option<crate::ui::animation::AnimationState>,
    pub render_cache: RenderCache,
}

impl WorktreeContext {
    pub fn new(path: PathBuf, repo: &Repository) -> Self {
        let branch_label = resolve_branch_label(repo);
        Self {
            repo_path: path,
            branch_label,
            mode: DiffMode::WorkingTree,
            files: Vec::new(),
            active_file: 0,
            scroll_offset: 0,
            collapse_level: CollapseLevel::Scoped,
            animation: None,
            render_cache: RenderCache::new(),
        }
    }

    pub fn active_file(&self) -> Option<&FileDiff> {
        self.files.get(self.active_file)
    }

    /// Recompute diffs for this worktree, preserving active file selection.
    pub fn recompute(&mut self, provider: &Git2Provider) -> Result<()> {
        let mut prev_paths: Vec<PathBuf> = Vec::new();
        if let Some(prev_path) = self.active_file().map(|f| f.path.clone()) {
            prev_paths.push(prev_path);
        }
        if let Some(prev_old_path) = self.active_file().and_then(|f| f.old_path.clone()) {
            if !prev_paths.contains(&prev_old_path) {
                prev_paths.push(prev_old_path);
            }
        }

        self.files = provider.compute_diff(&self.repo_path, self.mode)?;
        self.render_cache.invalidate();

        if let Ok(repo) = Repository::discover(&self.repo_path) {
            self.branch_label = resolve_branch_label(&repo);
        }

        if self.files.is_empty() {
            self.active_file = 0;
            self.scroll_offset = 0;
            return Ok(());
        }

        let new_index = if !prev_paths.is_empty() {
            self.files
                .iter()
                .position(|f| {
                    prev_paths.iter().any(|path| {
                        f.path == *path || f.old_path.as_deref() == Some(path.as_path())
                    })
                })
                .unwrap_or(0)
        } else {
            0
        };
        self.active_file = new_index;

        Ok(())
    }

    pub fn clamp_scroll_offset(&mut self, total_lines: usize, visible_height: usize) {
        if visible_height == 0 || total_lines == 0 {
            self.scroll_offset = 0;
            return;
        }

        let max_scroll = total_lines.saturating_sub(visible_height);
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
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

    /// Switch to a different diff mode, resetting file selection and scroll.
    /// Returns true if the mode actually changed (caller should recompute diffs).
    pub fn set_mode(&mut self, mode: DiffMode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        self.render_cache.invalidate();
        self.active_file = 0;
        self.scroll_offset = 0;
        true
    }

    pub fn cycle_collapse(&mut self) {
        self.collapse_level = match self.collapse_level {
            CollapseLevel::Tight => CollapseLevel::Scoped,
            CollapseLevel::Scoped => CollapseLevel::Expanded,
            CollapseLevel::Expanded => CollapseLevel::Tight,
        };
    }

    pub fn next_hunk_with_offsets(
        &mut self,
        hunk_starts: &[usize],
        total_lines: usize,
        visible_height: usize,
    ) {
        if total_lines == 0 || hunk_starts.is_empty() {
            self.scroll_offset = 0;
            return;
        }

        let next = hunk_starts
            .iter()
            .copied()
            .find(|&start| start > self.scroll_offset)
            .unwrap_or(*hunk_starts.first().expect("non-empty hunk_starts"));

        self.scroll_offset = next;

        if self.scroll_offset >= total_lines {
            self.scroll_offset = total_lines.saturating_sub(1);
        }

        self.clamp_scroll_offset(total_lines, visible_height);
    }

    pub fn prev_hunk_with_offsets(
        &mut self,
        hunk_starts: &[usize],
        total_lines: usize,
        visible_height: usize,
    ) {
        if total_lines == 0 || hunk_starts.is_empty() {
            self.scroll_offset = 0;
            return;
        }

        let prev = hunk_starts
            .iter()
            .rev()
            .copied()
            .find(|&start| start < self.scroll_offset)
            .unwrap_or(*hunk_starts.last().expect("non-empty hunk_starts"));

        self.scroll_offset = prev;

        if self.scroll_offset >= total_lines {
            self.scroll_offset = total_lines.saturating_sub(1);
        }

        self.clamp_scroll_offset(total_lines, visible_height);
    }

    pub fn scroll_page_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
    }

    pub fn scroll_page_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_to_bottom(&mut self, total_lines: usize, visible_height: usize) {
        if visible_height == 0 {
            self.scroll_offset = 0;
            return;
        }
        self.scroll_offset = total_lines.saturating_sub(visible_height);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }
}

pub struct App {
    pub contexts: Vec<WorktreeContext>,
    pub active_worktree: usize,
    pub should_quit: bool,
    pub manager: WorktreeManager,
}

impl App {
    pub fn active_context(&self) -> &WorktreeContext {
        &self.contexts[self.active_worktree]
    }

    pub fn active_context_mut(&mut self) -> &mut WorktreeContext {
        &mut self.contexts[self.active_worktree]
    }

    pub fn next_worktree(&mut self) {
        if self.contexts.len() > 1 {
            self.active_worktree = (self.active_worktree + 1) % self.contexts.len();
        }
    }

    /// Remove the context matching `path`, adjusting `active_worktree` to
    /// maintain the invariant `active_worktree < contexts.len()`.
    /// Returns the removed index so the caller can also remove the watcher.
    pub fn remove_context_by_path(&mut self, path: &std::path::Path) -> Option<usize> {
        let idx = self.contexts.iter().position(|c| c.repo_path == path)?;
        self.contexts.remove(idx);
        if self.contexts.is_empty() {
            self.active_worktree = 0;
        } else if self.active_worktree == idx {
            self.active_worktree = idx % self.contexts.len();
        } else if self.active_worktree > idx {
            self.active_worktree -= 1;
        }
        Some(idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::model::{FileStatus, Hunk};
    use git2::{Repository, Signature};
    use std::path::Path;

    fn setup_repo_with_commit() -> (tempfile::TempDir, Repository) {
        let tmp_dir = tempfile::tempdir().expect("create temp repo dir");
        let repo_path = tmp_dir.path().to_path_buf();
        let repo = Repository::init(&repo_path).expect("init test repo");

        let mut config = repo.config().expect("repo config");
        config
            .set_str("user.name", "better-diff-test")
            .expect("set test user.name");
        config
            .set_str("user.email", "test@example.com")
            .expect("set test user.email");

        std::fs::write(repo_path.join("README.md"), "test file\n").expect("write test file");

        let mut index = repo.index().expect("get index");
        index
            .add_path(Path::new("README.md"))
            .expect("add file to index");
        index.write().expect("write index");

        let tree_id = index.write_tree().expect("write tree");
        let sig = Signature::now("better-diff", "test@example.com").expect("signature");
        let _ = {
            let tree = repo.find_tree(tree_id).expect("load tree");
            repo.commit(
                Some("HEAD"),
                &sig,
                &sig,
                "initial commit",
                &tree,
                &[],
            )
            .expect("create initial commit")
        };

        (tmp_dir, repo)
    }

    fn test_context() -> WorktreeContext {
        WorktreeContext {
            repo_path: PathBuf::from("."),
            branch_label: String::from("test"),
            mode: DiffMode::WorkingTree,
            files: Vec::new(),
            active_file: 0,
            scroll_offset: 0,
            collapse_level: CollapseLevel::Scoped,
            animation: None,
            render_cache: RenderCache::new(),
        }
    }

    fn make_test_files(count: usize) -> Vec<FileDiff> {
        (0..count)
            .map(|i| FileDiff {
                path: PathBuf::from(format!("file{}.rs", i)),
                old_path: None,
                status: FileStatus::Modified,
                hunks: vec![Hunk {
                    old_start: 1,
                    new_start: 1,
                    old_lines: 1,
                    new_lines: 1,
                    lines: vec![],
                }],
                old_content: String::new(),
                new_content: String::new(),
                fold_regions: vec![],
                move_matches: vec![],
            })
            .collect()
    }

    #[test]
    fn test_file_navigation() {
        let mut ctx = test_context();
        ctx.files = make_test_files(3);

        // Start at file 0
        assert_eq!(ctx.active_file, 0);

        // next wraps forward
        ctx.next_file();
        assert_eq!(ctx.active_file, 1);
        ctx.next_file();
        assert_eq!(ctx.active_file, 2);
        ctx.next_file();
        assert_eq!(ctx.active_file, 0); // wraps around

        // prev wraps backward
        ctx.prev_file();
        assert_eq!(ctx.active_file, 2); // wraps backward from 0
        ctx.prev_file();
        assert_eq!(ctx.active_file, 1);

        // scroll_offset resets on file change
        ctx.scroll_offset = 42;
        ctx.next_file();
        assert_eq!(ctx.scroll_offset, 0);

        ctx.scroll_offset = 42;
        ctx.prev_file();
        assert_eq!(ctx.scroll_offset, 0);

        // select_file bounds-checks
        ctx.select_file(2);
        assert_eq!(ctx.active_file, 2);
        assert_eq!(ctx.scroll_offset, 0);

        ctx.select_file(99); // out of bounds, no change
        assert_eq!(ctx.active_file, 2);

        // Navigation with empty files does nothing
        let mut empty_ctx = test_context();
        empty_ctx.next_file();
        assert_eq!(empty_ctx.active_file, 0);
        empty_ctx.prev_file();
        assert_eq!(empty_ctx.active_file, 0);
    }

    #[test]
    fn test_set_mode() {
        let mut ctx = test_context();
        assert_eq!(ctx.mode, DiffMode::WorkingTree);

        // Switching to a different mode returns true
        assert!(ctx.set_mode(DiffMode::Staged));
        assert_eq!(ctx.mode, DiffMode::Staged);

        // Setting the same mode returns false
        assert!(!ctx.set_mode(DiffMode::Staged));

        // Switch back
        assert!(ctx.set_mode(DiffMode::WorkingTree));
        assert_eq!(ctx.mode, DiffMode::WorkingTree);
    }

    #[test]
    fn test_collapse_cycle() {
        let mut ctx = test_context();
        assert_eq!(ctx.collapse_level, CollapseLevel::Scoped); // default

        ctx.cycle_collapse();
        assert_eq!(ctx.collapse_level, CollapseLevel::Expanded);

        ctx.cycle_collapse();
        assert_eq!(ctx.collapse_level, CollapseLevel::Tight);

        ctx.cycle_collapse();
        assert_eq!(ctx.collapse_level, CollapseLevel::Scoped); // full cycle
    }

    #[test]
    fn test_hunk_navigation_clamps_to_visible_rows() {
        let mut ctx = test_context();
        ctx.files = make_test_files(1);
        ctx.scroll_offset = 50;

        ctx.next_hunk_with_offsets(&[10, 40], 45, 10);
        assert_eq!(ctx.scroll_offset, 10); // wrapped to first hunk

        ctx.prev_hunk_with_offsets(&[10, 40], 45, 10);
        assert_eq!(ctx.scroll_offset, 35); // previous hunk before clamped position
    }

    #[test]
    fn test_hunk_navigation_wraps_between_hunks() {
        let mut ctx = test_context();
        ctx.files = make_test_files(1);

        ctx.scroll_offset = 160;
        ctx.next_hunk_with_offsets(&[20, 60, 100], 140, 20);
        assert_eq!(ctx.scroll_offset, 20);

        ctx.scroll_offset = 1;
        ctx.prev_hunk_with_offsets(&[20, 60, 100], 140, 20);
        assert_eq!(ctx.scroll_offset, 100);
    }

    #[test]
    fn test_scroll_to_top_bottom_helpers() {
        let mut ctx = test_context();

        ctx.scroll_offset = 15;
        ctx.scroll_to_top();
        assert_eq!(ctx.scroll_offset, 0);

        ctx.scroll_to_bottom(100, 15);
        assert_eq!(ctx.scroll_offset, 85);

        ctx.scroll_to_bottom(10, 20);
        assert_eq!(ctx.scroll_offset, 0);
    }

    #[test]
    fn test_layout_cache_reuse_by_collapse_and_file() {
        let mut ctx = test_context();
        ctx.files = make_test_files(1);

        ctx.render_cache.cached_file_index = Some(0);
        ctx.render_cache.ensure_layout(0, &ctx.files[0], ctx.collapse_level);
        let first_builds = ctx.render_cache.layout_rebuild_count;

        ctx.render_cache.ensure_layout(0, &ctx.files[0], ctx.collapse_level);
        assert_eq!(
            ctx.render_cache.layout_rebuild_count,
            first_builds,
            "layout should be reused when file and collapse level are unchanged"
        );

        ctx.cycle_collapse();
        ctx.render_cache.ensure_layout(0, &ctx.files[0], ctx.collapse_level);
        assert_eq!(
            ctx.render_cache.layout_rebuild_count,
            first_builds + 1,
            "layout should rebuild after collapse level changes"
        );
    }

    #[test]
    fn test_hunk_navigation_no_lines_or_invalid_viewport() {
        let mut ctx = test_context();
        ctx.scroll_offset = 42;

        ctx.next_hunk_with_offsets(&[0], 0, 20);
        assert_eq!(ctx.scroll_offset, 0);

        ctx.scroll_offset = 42;
        ctx.prev_hunk_with_offsets(&[0], 3, 20);
        assert_eq!(ctx.scroll_offset, 0); // visible window larger than content
    }

    #[test]
    fn test_resolve_branch_label_attached() {
        let (_tmp_dir, repo) = setup_repo_with_commit();
        let branch_name = repo
            .head()
            .expect("get HEAD")
            .shorthand()
            .unwrap_or("unknown")
            .to_string();

        assert_eq!(resolve_branch_label(&repo), branch_name);
    }

    #[test]
    fn test_resolve_branch_label_detached() {
        let (_tmp_dir, repo) = setup_repo_with_commit();
        let head = repo.head().expect("get HEAD");
        let oid = head.target().expect("head target");
        drop(head);
        repo.set_head_detached(oid).expect("set detached");

        let short_oid = oid.to_string();
        let expected = format!("detached@{}", &short_oid[..8]);
        assert_eq!(resolve_branch_label(&repo), expected);
    }

    #[test]
    fn test_resolve_branch_label_unknown_on_empty_repo() {
        let tmp_dir = tempfile::tempdir().expect("create temp dir");
        let repo = Repository::init(tmp_dir.path()).expect("init repo");
        assert_eq!(resolve_branch_label(&repo), "unknown");
    }

    #[test]
    fn test_worktree_context_new_resolves_branch() {
        let (_tmp_dir, repo) = setup_repo_with_commit();
        let workdir = repo.workdir().expect("has workdir").to_path_buf();
        let ctx = WorktreeContext::new(workdir, &repo);
        assert_ne!(ctx.branch_label, "unknown");
        assert_eq!(ctx.mode, DiffMode::WorkingTree);
        assert!(ctx.files.is_empty());
    }
}
