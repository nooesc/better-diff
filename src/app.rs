use std::path::PathBuf;

use crate::diff::model::{CollapseLevel, DiffMode, FileDiff};
use crate::syntax::HighlightSpan;
use git2::Repository;

/// Cached syntax highlights for a single file, keyed by file index.
#[derive(Default)]
pub struct RenderCache {
    /// The file index these highlights were computed for.
    pub cached_file_index: Option<usize>,
    pub old_highlights: Vec<Vec<HighlightSpan>>,
    pub new_highlights: Vec<Vec<HighlightSpan>>,
}

impl RenderCache {
    pub fn new() -> Self {
        Self {
            cached_file_index: None,
            old_highlights: Vec::new(),
            new_highlights: Vec::new(),
        }
    }

    /// Invalidate the cache so highlights are recomputed on next render.
    pub fn invalidate(&mut self) {
        self.cached_file_index = None;
        self.old_highlights.clear();
        self.new_highlights.clear();
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

        let mut next = None;
        for &start in hunk_starts {
            if start > self.scroll_offset {
                next = Some(start);
                break;
            }
        }

        self.scroll_offset = next
            .unwrap_or(*hunk_starts.last().expect("non-empty hunk_starts"));

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

        let mut prev = None;
        for &start in hunk_starts.iter().rev() {
            if start < self.scroll_offset {
                prev = Some(start);
                break;
            }
        }

        self.scroll_offset = prev.unwrap_or(*hunk_starts.first().expect("non-empty hunk_starts"));

        if self.scroll_offset >= total_lines {
            self.scroll_offset = total_lines.saturating_sub(1);
        }

        self.clamp_scroll_offset(total_lines, visible_height);
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
        assert_eq!(app.scroll_offset, 35); // clamped by total 45 with 10 visible rows

        app.prev_hunk_with_offsets(&[10, 40], 45, 10);
        assert_eq!(app.scroll_offset, 10); // previous hunk before clamped position
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
}
