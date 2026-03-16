use std::path::PathBuf;

use crate::diff::model::{CollapseLevel, DiffMode, FileDiff};
use crate::syntax::HighlightSpan;
use crate::ui::split_pane::RenderedFileLayout;
use git2::Repository;

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

pub struct App {
    pub mode: DiffMode,
    pub files: Vec<FileDiff>,
    pub active_file: usize,
    pub collapse_level: CollapseLevel,
    pub scroll_offset: usize,
    pub branch_label: String,
    pub should_quit: bool,
    pub repo_path: PathBuf,
    pub animation: Option<crate::ui::animation::AnimationState>,
    pub render_cache: RenderCache,
}

impl App {
    pub fn new(repo_path: PathBuf) -> Self {
        Self {
            mode: DiffMode::WorkingTree,
            files: Vec::new(),
            active_file: 0,
            collapse_level: CollapseLevel::Scoped,
            scroll_offset: 0,
            branch_label: String::from("unknown"),
            should_quit: false,
            repo_path,
            animation: None,
            render_cache: RenderCache::new(),
        }
    }

    pub fn resolve_branch_label(&self) -> String {
        let repo = match Repository::discover(&self.repo_path) {
            Ok(repo) => repo,
            Err(_) => return String::from("unknown"),
        };

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

    pub fn active_file(&self) -> Option<&FileDiff> {
        self.files.get(self.active_file)
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
        let mut app = App::new(PathBuf::from("."));
        app.files = make_test_files(3);

        // Start at file 0
        assert_eq!(app.active_file, 0);

        // next wraps forward
        app.next_file();
        assert_eq!(app.active_file, 1);
        app.next_file();
        assert_eq!(app.active_file, 2);
        app.next_file();
        assert_eq!(app.active_file, 0); // wraps around

        // prev wraps backward
        app.prev_file();
        assert_eq!(app.active_file, 2); // wraps backward from 0
        app.prev_file();
        assert_eq!(app.active_file, 1);

        // scroll_offset resets on file change
        app.scroll_offset = 42;
        app.next_file();
        assert_eq!(app.scroll_offset, 0);

        app.scroll_offset = 42;
        app.prev_file();
        assert_eq!(app.scroll_offset, 0);

        // select_file bounds-checks
        app.select_file(2);
        assert_eq!(app.active_file, 2);
        assert_eq!(app.scroll_offset, 0);

        app.select_file(99); // out of bounds, no change
        assert_eq!(app.active_file, 2);

        // Navigation with empty files does nothing
        let mut empty_app = App::new(PathBuf::from("."));
        empty_app.next_file();
        assert_eq!(empty_app.active_file, 0);
        empty_app.prev_file();
        assert_eq!(empty_app.active_file, 0);
    }

    #[test]
    fn test_set_mode() {
        let mut app = App::new(PathBuf::from("."));
        assert_eq!(app.mode, DiffMode::WorkingTree);

        // Switching to a different mode returns true
        assert!(app.set_mode(DiffMode::Staged));
        assert_eq!(app.mode, DiffMode::Staged);

        // Setting the same mode returns false
        assert!(!app.set_mode(DiffMode::Staged));

        // Switch back
        assert!(app.set_mode(DiffMode::WorkingTree));
        assert_eq!(app.mode, DiffMode::WorkingTree);
    }

    #[test]
    fn test_collapse_cycle() {
        let mut app = App::new(PathBuf::from("."));
        assert_eq!(app.collapse_level, CollapseLevel::Scoped); // default

        app.cycle_collapse();
        assert_eq!(app.collapse_level, CollapseLevel::Expanded);

        app.cycle_collapse();
        assert_eq!(app.collapse_level, CollapseLevel::Tight);

        app.cycle_collapse();
        assert_eq!(app.collapse_level, CollapseLevel::Scoped); // full cycle
    }

    #[test]
    fn test_hunk_navigation_clamps_to_visible_rows() {
        let mut app = App::new(PathBuf::from("."));
        app.files = make_test_files(1);
        app.scroll_offset = 50;

        app.next_hunk_with_offsets(&[10, 40], 45, 10);
        assert_eq!(app.scroll_offset, 10); // wrapped to first hunk

        app.prev_hunk_with_offsets(&[10, 40], 45, 10);
        assert_eq!(app.scroll_offset, 35); // previous hunk before clamped position
    }

    #[test]
    fn test_hunk_navigation_wraps_between_hunks() {
        let mut app = App::new(PathBuf::from("."));
        app.files = make_test_files(1);

        app.scroll_offset = 160;
        app.next_hunk_with_offsets(&[20, 60, 100], 140, 20);
        assert_eq!(app.scroll_offset, 20);

        app.scroll_offset = 1;
        app.prev_hunk_with_offsets(&[20, 60, 100], 140, 20);
        assert_eq!(app.scroll_offset, 100);
    }

    #[test]
    fn test_scroll_to_top_bottom_helpers() {
        let mut app = App::new(PathBuf::from("."));

        app.scroll_offset = 15;
        app.scroll_to_top();
        assert_eq!(app.scroll_offset, 0);

        app.scroll_to_bottom(100, 15);
        assert_eq!(app.scroll_offset, 85);

        app.scroll_to_bottom(10, 20);
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_layout_cache_reuse_by_collapse_and_file() {
        let mut app = App::new(PathBuf::from("."));
        app.files = make_test_files(1);

        app.render_cache.cached_file_index = Some(0);
        app.render_cache.ensure_layout(0, &app.files[0], app.collapse_level);
        let first_builds = app.render_cache.layout_rebuild_count;

        app.render_cache.ensure_layout(0, &app.files[0], app.collapse_level);
        assert_eq!(
            app.render_cache.layout_rebuild_count,
            first_builds,
            "layout should be reused when file and collapse level are unchanged"
        );

        app.cycle_collapse();
        app.render_cache.ensure_layout(0, &app.files[0], app.collapse_level);
        assert_eq!(
            app.render_cache.layout_rebuild_count,
            first_builds + 1,
            "layout should rebuild after collapse level changes"
        );
    }

    #[test]
    fn test_hunk_navigation_no_lines_or_invalid_viewport() {
        let mut app = App::new(PathBuf::from("."));
        app.scroll_offset = 42;

        app.next_hunk_with_offsets(&[0], 0, 20);
        assert_eq!(app.scroll_offset, 0);

        app.scroll_offset = 42;
        app.prev_hunk_with_offsets(&[0], 3, 20);
        assert_eq!(app.scroll_offset, 0); // visible window larger than content
    }

    #[test]
    fn test_resolve_branch_label_attached() {
        let (_tmp_dir, repo) = setup_repo_with_commit();
        let app = App::new(repo.path().to_path_buf());
        let branch_name = repo
            .head()
            .expect("get HEAD")
            .shorthand()
            .unwrap_or("unknown")
            .to_string();

        assert_eq!(app.resolve_branch_label(), branch_name);
    }

    #[test]
    fn test_resolve_branch_label_detached() {
        let (_tmp_dir, repo) = setup_repo_with_commit();
        let head = repo.head().expect("get HEAD");
        let oid = head.target().expect("head target");
        repo.set_head_detached(oid).expect("set detached");

        let app = App::new(repo.path().to_path_buf());
        let short_oid = oid.to_string();
        let expected = format!("detached@{}", &short_oid[..8]);
        assert_eq!(app.resolve_branch_label(), expected);
    }

    #[test]
    fn test_resolve_branch_label_unknown_on_non_repo() {
        let tmp_dir = tempfile::tempdir().expect("create temp path");
        let app = App::new(tmp_dir.path().to_path_buf());

        assert_eq!(app.resolve_branch_label(), "unknown");
    }
}
