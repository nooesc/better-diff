use std::path::PathBuf;

use crate::diff::model::{CollapseLevel, DiffMode, FileDiff};
use crate::syntax::HighlightSpan;

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
            should_quit: false,
            repo_path,
            animation: None,
            render_cache: RenderCache::new(),
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

    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    /// Jump scroll_offset to the start of the next hunk in the active file.
    pub fn next_hunk(&mut self) {
        if let Some(file) = self.files.get(self.active_file) {
            // Compute cumulative line offsets for each hunk
            let mut offset = 0usize;
            for hunk in &file.hunks {
                if offset > self.scroll_offset {
                    self.scroll_offset = offset;
                    self.animation = Some(crate::ui::animation::AnimationState::new());
                    return;
                }
                offset += hunk.lines.len();
            }
            // If we didn't find a next hunk, stay where we are
        }
    }

    /// Jump scroll_offset to the start of the previous hunk in the active file.
    pub fn prev_hunk(&mut self) {
        if let Some(file) = self.files.get(self.active_file) {
            // Compute cumulative line offsets for each hunk, find the one before current
            let mut offsets: Vec<usize> = Vec::new();
            let mut offset = 0usize;
            for hunk in &file.hunks {
                offsets.push(offset);
                offset += hunk.lines.len();
            }
            // Find the last hunk offset that is strictly less than current scroll_offset
            for &o in offsets.iter().rev() {
                if o < self.scroll_offset {
                    self.scroll_offset = o;
                    self.animation = Some(crate::ui::animation::AnimationState::new());
                    return;
                }
            }
            // If we're at or before the first hunk, go to 0
            self.scroll_offset = 0;
        }
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
}
