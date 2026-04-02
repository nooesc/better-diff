pub mod animation;
pub mod minimap;
pub mod split_pane;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
};

use crate::app::{App, WorktreeContext};
use crate::diff::model::{DiffMode, FileStatus, FileDiff};
use crate::syntax::highlight_file;
use std::collections::HashMap;
use std::path::Path;

pub fn ensure_active_file_layout(ctx: &mut WorktreeContext) -> bool {
    let file_index = ctx.active_file;
    let file = match ctx.files.get(file_index) {
        Some(file) => file,
        None => return false,
    };

    if ctx.render_cache.cached_file_index != Some(file_index) {
        let (old_highlight_path, new_highlight_path) = highlight_path_pair(file);

        ctx.render_cache.old_highlights = highlight_file(old_highlight_path, &file.old_content);
        ctx.render_cache.new_highlights = highlight_file(new_highlight_path, &file.new_content);
        ctx.render_cache.cached_file_index = Some(file_index);
    }

    ctx.render_cache.ensure_layout(file_index, file, ctx.collapse_level);
    true
}

pub fn render(frame: &mut Frame, app: &mut App) {
    let worktree_count = app.contexts.len();
    let active_worktree_index = app.active_worktree;
    let ctx = app.active_context_mut();

    let [tab_area, mode_area, content_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // --- Tab bar ---
    if ctx.files.is_empty() {
        let no_changes = Paragraph::new(" No changes detected")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(no_changes, tab_area);
    } else {
        let base_name_of = |f: &FileDiff| -> String {
            f.path
                .file_name()
                .map_or_else(
                    || f.path.to_string_lossy().to_string(),
                    |name| name.to_string_lossy().to_string(),
                )
        };

        let mut name_counts: HashMap<String, usize> = HashMap::new();
        for file in &ctx.files {
            *name_counts.entry(base_name_of(file)).or_insert(0) += 1;
        }

        let titles: Vec<String> = ctx
            .files
            .iter()
            .map(|f| {
                let base_name = base_name_of(f);
                let name = if name_counts.get(&base_name).is_some_and(|count| *count > 1) {
                    f.path.to_string_lossy().to_string()
                } else {
                    base_name
                };
                file_title_with_status(f, &name)
            })
            .collect();
        let tabs = Tabs::new(titles)
            .select(ctx.active_file)
            .style(Style::default().fg(Color::DarkGray))
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .divider("│");
        frame.render_widget(tabs, tab_area);
    }

    // --- Mode indicator ---
    let mode_label = match ctx.mode {
        DiffMode::WorkingTree => " [Working Tree]",
        DiffMode::Staged => " [Staged]",
    };
    let branch_label = if worktree_count > 1 {
        let wt_path = substitute_home(&ctx.repo_path);
        format!(
            " [{}/{}] {} [{}]",
            active_worktree_index + 1,
            worktree_count,
            ctx.branch_label,
            wt_path,
        )
    } else {
        format!(" [{}]", ctx.branch_label)
    };
    let file_count = format!("  {} file(s)", ctx.files.len());
    let mode_line = Line::from(vec![
        Span::styled(mode_label, Style::default().fg(Color::Cyan)),
        Span::styled(branch_label, Style::default().fg(Color::Cyan)),
        Span::styled(file_count, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(mode_line), mode_area);

    // --- Content area ---
    if ctx.files.get(ctx.active_file).is_some() && ensure_active_file_layout(ctx) {
        let layout = &ctx.render_cache.layout;
        let file = &ctx.files[ctx.active_file];
        split_pane::render_split_pane(
            frame,
            content_area,
            file,
            ctx.scroll_offset,
            ctx.animation.as_ref(),
            layout,
        );
    } else {
        let content = Paragraph::new("No changes to display").block(
            Block::default()
                .borders(Borders::ALL)
                .title("Diff View"),
        );
        frame.render_widget(content, content_area);
    }

    // --- Status bar ---
    let key = Style::default().fg(Color::Yellow);
    let dim = Style::default().fg(Color::DarkGray);
    let mut status_spans = vec![
        Span::styled(" [q]", key), Span::styled("uit ", dim),
        Span::styled("[Tab]", key), Span::styled(" next file ", dim),
        Span::styled("[PgUp/PgDn]", key), Span::styled(" page ", dim),
        Span::styled("[g/G]", key), Span::styled(" top/bottom ", dim),
        Span::styled("[s]", key), Span::styled(" staged ", dim),
        Span::styled("[w]", key), Span::styled("orking tree ", dim),
        Span::styled("[n/N]", key), Span::styled(" hunks ", dim),
        Span::styled("[c]", key), Span::styled("ollapse", dim),
    ];
    if worktree_count > 1 {
        status_spans.push(Span::styled(" ]", key));
        status_spans.push(Span::styled(" wt", dim));
    }
    let status_line = Line::from(status_spans);
    frame.render_widget(Paragraph::new(status_line), status_area);
}

fn file_status_label(status: FileStatus) -> &'static str {
    match status {
        FileStatus::Added => "[+]",
        FileStatus::Deleted => "[-]",
        FileStatus::Renamed => "[R]",
        FileStatus::Modified => "[ ]",
    }
}

fn file_title_with_status(file: &FileDiff, display_name: &str) -> String {
    let status = file_status_label(file.status);

    if file.status == FileStatus::Renamed {
        if let Some(old_path) = &file.old_path {
            return format!(
                "{} {} ← {}",
                status,
                display_name,
                old_path.to_string_lossy()
            );
        }
    }

    format!("{}{}", status, display_name)
}

fn substitute_home(path: &Path) -> String {
    if let Ok(home) = std::env::var("HOME") {
        let path_str = path.to_string_lossy();
        if let Some(rest) = path_str.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    path.to_string_lossy().to_string()
}

fn highlight_path_pair(file: &FileDiff) -> (&Path, &Path) {
    (
        file.old_path.as_deref().unwrap_or(&file.path),
        &file.path,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::model::{FileDiff, Hunk};
    use std::path::PathBuf;

    fn make_renamed_file() -> FileDiff {
        FileDiff {
            path: PathBuf::from("src/new_name.rs"),
            old_path: Some(PathBuf::from("src/old_name.rs")),
            status: FileStatus::Renamed,
            hunks: vec![Hunk {
                old_start: 1,
                new_start: 1,
                old_lines: 0,
                new_lines: 0,
                lines: vec![],
            }],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }
    }

    fn make_modified_file() -> FileDiff {
        FileDiff {
            path: PathBuf::from("src/main.rs"),
            old_path: None,
            status: FileStatus::Modified,
            hunks: vec![Hunk {
                old_start: 1,
                new_start: 1,
                old_lines: 0,
                new_lines: 0,
                lines: vec![],
            }],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }
    }

    #[test]
    fn test_file_title_with_status_renamed_shows_old_path() {
        let file = make_renamed_file();
        let title = file_title_with_status(&file, "new_name.rs");
        assert_eq!(title, "[R] new_name.rs ← src/old_name.rs");
    }

    #[test]
    fn test_highlight_path_pair_uses_old_path_for_old_version_on_rename() {
        let renamed = make_renamed_file();
        let (old_path, new_path) = highlight_path_pair(&renamed);
        assert_eq!(old_path, Path::new("src/old_name.rs"));
        assert_eq!(new_path, Path::new("src/new_name.rs"));
    }

    #[test]
    fn test_highlight_path_pair_uses_new_path_when_old_path_missing() {
        let file = make_modified_file();
        let (old_path, new_path) = highlight_path_pair(&file);
        assert_eq!(old_path, Path::new("src/main.rs"));
        assert_eq!(new_path, Path::new("src/main.rs"));
    }
}
