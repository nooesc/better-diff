use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::diff::model::{FileDiff, LineKind};

/// A heat-map minimap widget that shows change density across a file.
///
/// Each row of the minimap represents a proportional region of the file's diff lines.
/// Rows are colored by density: hot (many changes), warm, cool, or empty.
/// A scroll position indicator shows where the current viewport is.
pub struct Minimap {
    scroll_offset: usize,
    visible_height: usize,
    total_lines: usize,
    changed_lines: Vec<bool>,
}

impl Minimap {
    pub fn new(file: &FileDiff, scroll_offset: usize, visible_height: usize) -> Self {
        let changed_lines = build_changed_lines(file);
        let total_lines = changed_lines.len();

        Self {
            scroll_offset,
            visible_height,
            total_lines,
            changed_lines,
        }
    }

    /// Build a minimap from precomputed rendered-line metadata.
    pub fn with_rendered_lines(
        scroll_offset: usize,
        visible_height: usize,
        mut changed_lines: Vec<bool>,
        total_lines: usize,
    ) -> Self {
        if changed_lines.len() < total_lines {
            changed_lines.resize(total_lines, false);
        } else if changed_lines.len() > total_lines {
            changed_lines.truncate(total_lines);
        }

        Self {
            scroll_offset,
            visible_height,
            total_lines,
            changed_lines,
        }
    }

    /// Count total lines across all hunks (including hunk headers).
    fn total_lines(&self) -> usize {
        self.total_lines
    }

    /// Build a density map: for each row of the minimap area, calculate the fraction
    /// of changed lines (Added/Modified/Deleted) in that proportional region.
    fn build_density_map(&self, rows: usize) -> Vec<f64> {
        if rows == 0 {
            return vec![];
        }

        let total = self.total_lines();
        if total == 0 {
            return vec![0.0; rows];
        }
        let changed = self.changed_lines.as_slice();

        let mut densities = Vec::with_capacity(rows);
        for row in 0..rows {
            let start = (row * total) / rows;
            let end = ((row + 1) * total) / rows;
            if start >= end {
                densities.push(0.0);
                continue;
            }
            let region = &changed[start..end];
            let change_count = region.iter().filter(|&&c| c).count();
            densities.push(change_count as f64 / region.len() as f64);
        }

        // Normalize to 0.0..1.0 relative to the maximum density.
        let max_density = densities.iter().cloned().fold(0.0_f64, f64::max);
        if max_density > 0.0 {
            for d in &mut densities {
                *d /= max_density;
            }
        }

        densities
    }
}

fn build_changed_lines(file: &FileDiff) -> Vec<bool> {
    let mut changed = Vec::new();
    for hunk in &file.hunks {
        // hunk headers are not changed lines in minimap visualization.
        changed.push(false);
        changed.extend(hunk.lines.iter().map(|line| {
            matches!(
                line.kind,
                LineKind::Added | LineKind::Deleted | LineKind::Modified
            )
        }));
    }
    changed
}

impl Widget for Minimap {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let height = area.height as usize;
        if height == 0 || area.width == 0 {
            return;
        }

        let total = self.total_lines();
        let densities = self.build_density_map(height);

        // Determine scroll indicator rows.
        let (indicator_start, indicator_end) = if total > 0 {
            let start_row = (self.scroll_offset * height) / total.max(1);
            let end_row = ((self.scroll_offset + self.visible_height) * height) / total.max(1);
            let end_row = end_row.min(height);
            (start_row, end_row)
        } else {
            (0, height)
        };

        for (i, &density) in densities.iter().enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }

            let (symbol, fg) = if density > 0.0 {
                let color = if density > 0.7 {
                    Color::Rgb(255, 100, 50) // Hot
                } else if density > 0.3 {
                    Color::Rgb(200, 150, 50) // Warm
                } else {
                    Color::Rgb(80, 80, 40) // Cool
                };
                ("\u{2588}", color) // "█"
            } else {
                ("\u{2595}", Color::Rgb(30, 30, 30)) // "▕"
            };

            let in_viewport = i >= indicator_start && i < indicator_end;
            let bg = if in_viewport {
                Color::Rgb(60, 60, 80)
            } else {
                Color::Reset
            };

            let style = Style::default().fg(fg).bg(bg);

            // Fill all columns in the minimap area with the symbol.
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_symbol(symbol).set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::model::{DiffLine, FileStatus, Hunk, LineKind};
    use std::path::PathBuf;

    fn make_test_file() -> FileDiff {
        FileDiff {
            path: PathBuf::from("test.rs"),
            old_path: None,
            status: FileStatus::Modified,
            hunks: vec![Hunk {
                old_start: 1,
                new_start: 1,
                old_lines: 3,
                new_lines: 3,
                lines: vec![
                    DiffLine {
                        kind: LineKind::Context,
                        old_line_no: Some(1),
                        new_line_no: Some(1),
                        old_text: Some("use std::io;".to_string()),
                        new_text: Some("use std::io;".to_string()),
                        tokens: vec![],
                    },
                    DiffLine {
                        kind: LineKind::Deleted,
                        old_line_no: Some(2),
                        new_line_no: None,
                        old_text: Some("let x = 1;".to_string()),
                        new_text: None,
                        tokens: vec![],
                    },
                    DiffLine {
                        kind: LineKind::Added,
                        old_line_no: None,
                        new_line_no: Some(2),
                        old_text: None,
                        new_text: Some("let y = 2;".to_string()),
                        tokens: vec![],
                    },
                ],
            }],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }
    }

    fn make_empty_file() -> FileDiff {
        FileDiff {
            path: PathBuf::from("empty.rs"),
            old_path: None,
            status: FileStatus::Modified,
            hunks: vec![],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }
    }

    #[test]
    fn test_total_lines() {
        let file = make_test_file();
        let minimap = Minimap::new(&file, 0, 10);
        // 1 hunk header + 3 lines = 4
        assert_eq!(minimap.total_lines(), 4);
    }

    #[test]
    fn test_total_lines_empty() {
        let file = make_empty_file();
        let minimap = Minimap::new(&file, 0, 10);
        assert_eq!(minimap.total_lines(), 0);
    }

    #[test]
    fn test_density_map_nonempty() {
        let file = make_test_file();
        let minimap = Minimap::new(&file, 0, 10);
        let densities = minimap.build_density_map(4);
        assert_eq!(densities.len(), 4);

        // Row 0 maps to line 0 (hunk header, not changed) => 0.0
        assert_eq!(densities[0], 0.0);
        // Row 1 maps to line 1 (context, not changed) => 0.0
        assert_eq!(densities[1], 0.0);
        // Row 2 maps to line 2 (deleted, changed) => 1.0 (normalized)
        assert!(densities[2] > 0.0);
        // Row 3 maps to line 3 (added, changed) => 1.0 (normalized)
        assert!(densities[3] > 0.0);
    }

    #[test]
    fn test_density_map_empty_file() {
        let file = make_empty_file();
        let minimap = Minimap::new(&file, 0, 10);
        let densities = minimap.build_density_map(5);
        assert_eq!(densities.len(), 5);
        for d in &densities {
            assert_eq!(*d, 0.0);
        }
    }

    #[test]
    fn test_density_map_zero_rows() {
        let file = make_test_file();
        let minimap = Minimap::new(&file, 0, 10);
        let densities = minimap.build_density_map(0);
        assert!(densities.is_empty());
    }

    #[test]
    fn test_render_does_not_panic() {
        let file = make_test_file();
        let minimap = Minimap::new(&file, 0, 10);
        let area = Rect::new(0, 0, 2, 10);
        let mut buf = Buffer::empty(area);
        minimap.render(area, &mut buf);
    }

    #[test]
    fn test_render_zero_area() {
        let file = make_test_file();
        let minimap = Minimap::new(&file, 0, 10);
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        minimap.render(area, &mut buf);
    }
}
