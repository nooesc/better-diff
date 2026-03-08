pub mod minimap;
pub mod split_pane;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
};

use crate::app::App;
use crate::diff::model::DiffMode;

pub fn render(frame: &mut Frame, app: &App) {
    let [tab_area, mode_area, content_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // --- Tab bar ---
    if app.files.is_empty() {
        let no_changes = Paragraph::new("No changes detected")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(no_changes, tab_area);
    } else {
        let titles: Vec<String> = app
            .files
            .iter()
            .map(|f| {
                f.path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| f.path.to_string_lossy().to_string())
            })
            .collect();
        let tabs = Tabs::new(titles)
            .select(app.active_file)
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .divider("|");
        frame.render_widget(tabs, tab_area);
    }

    // --- Mode indicator ---
    let mode_label = match app.mode {
        DiffMode::WorkingTree => "[Working Tree]",
        DiffMode::Staged => "[Staged]",
    };
    let file_count = format!("  {} file(s)", app.files.len());
    let mode_line = Line::from(vec![
        Span::styled(mode_label, Style::default().fg(Color::Cyan)),
        Span::raw(file_count),
    ]);
    frame.render_widget(Paragraph::new(mode_line), mode_area);

    // --- Content area ---
    if let Some(file) = app.active_file() {
        split_pane::render_split_pane(frame, content_area, file, app.scroll_offset);
    } else {
        let content = Paragraph::new("No changes to display").block(
            Block::default()
                .borders(Borders::ALL)
                .title("Diff View"),
        );
        frame.render_widget(content, content_area);
    }

    // --- Status bar ---
    let status_line = Line::from(vec![
        Span::styled("[q]", Style::default().fg(Color::Yellow)),
        Span::raw("uit "),
        Span::styled("[Tab]", Style::default().fg(Color::Yellow)),
        Span::raw(" next file "),
        Span::styled("[s]", Style::default().fg(Color::Yellow)),
        Span::raw("taged "),
        Span::styled("[w]", Style::default().fg(Color::Yellow)),
        Span::raw("orking tree "),
        Span::styled("[n/N]", Style::default().fg(Color::Yellow)),
        Span::raw(" hunks "),
        Span::styled("[c]", Style::default().fg(Color::Yellow)),
        Span::raw("ollapse"),
    ]);
    frame.render_widget(
        Paragraph::new(status_line).style(Style::default().bg(Color::DarkGray)),
        status_area,
    );
}
