use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::diff::model::{ChangeKind, DiffLine, FileDiff, LineKind};

/// Render a side-by-side diff view with the old file on the left and the new file on the right.
pub fn render_split_pane(
    frame: &mut Frame,
    area: Rect,
    file: &FileDiff,
    scroll_offset: usize,
) {
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas(area);

    let (old_lines, new_lines) = build_side_by_side_lines(file);

    let visible_height = left_area.height.saturating_sub(2) as usize; // subtract 2 for borders

    let old_visible: Vec<Line> = old_lines
        .into_iter()
        .skip(scroll_offset)
        .take(visible_height)
        .collect();

    let new_visible: Vec<Line> = new_lines
        .into_iter()
        .skip(scroll_offset)
        .take(visible_height)
        .collect();

    let file_path = file.path.to_string_lossy().to_string();

    let left_block = Block::default()
        .borders(Borders::ALL)
        .title(format!("old: {}", file_path));
    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(format!("new: {}", file_path));

    let left_paragraph = Paragraph::new(old_visible).block(left_block);
    let right_paragraph = Paragraph::new(new_visible).block(right_block);

    frame.render_widget(left_paragraph, left_area);
    frame.render_widget(right_paragraph, right_area);
}

/// Build parallel line lists for left (old) and right (new) panes from the file's hunks.
fn build_side_by_side_lines(file: &FileDiff) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let mut old_lines: Vec<Line<'static>> = Vec::new();
    let mut new_lines: Vec<Line<'static>> = Vec::new();

    for hunk in &file.hunks {
        // Hunk header line on both sides
        let header = format!(
            "@@ -{},{} +{},{} @@",
            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
        );
        let header_style = Style::default().fg(Color::DarkGray);
        old_lines.push(Line::from(Span::styled(header.clone(), header_style)));
        new_lines.push(Line::from(Span::styled(header, header_style)));

        let mut i = 0;
        while i < hunk.lines.len() {
            let line = &hunk.lines[i];

            match line.kind {
                LineKind::Context => {
                    let line_no = format_line_no(line.old_line_no);
                    let text = line.old_text.as_deref().unwrap_or("");
                    let old_line = Line::from(vec![
                        Span::styled(line_no.clone(), Style::default().fg(Color::DarkGray)),
                        Span::raw(text.to_string()),
                    ]);

                    let new_line_no = format_line_no(line.new_line_no);
                    let new_line = Line::from(vec![
                        Span::styled(new_line_no, Style::default().fg(Color::DarkGray)),
                        Span::raw(text.to_string()),
                    ]);

                    old_lines.push(old_line);
                    new_lines.push(new_line);
                    i += 1;
                }

                LineKind::Added => {
                    // Empty line on left, green text on right
                    old_lines.push(Line::from(Span::raw(String::new())));

                    let line_no = format_line_no(line.new_line_no);
                    let text = line.new_text.as_deref().unwrap_or("");
                    let new_line = Line::from(vec![
                        Span::styled(line_no, Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            text.to_string(),
                            Style::default()
                                .fg(Color::Green)
                                .bg(Color::Rgb(0, 40, 0)),
                        ),
                    ]);

                    new_lines.push(new_line);
                    i += 1;
                }

                LineKind::Deleted => {
                    // Red text on left, empty line on right
                    let line_no = format_line_no(line.old_line_no);
                    let text = line.old_text.as_deref().unwrap_or("");
                    let old_line = Line::from(vec![
                        Span::styled(line_no, Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            text.to_string(),
                            Style::default()
                                .fg(Color::Red)
                                .bg(Color::Rgb(40, 0, 0)),
                        ),
                    ]);

                    old_lines.push(old_line);
                    new_lines.push(Line::from(Span::raw(String::new())));
                    i += 1;
                }

                LineKind::Modified => {
                    // Modified lines come in pairs: first has old_tokens, second has new_tokens
                    let has_tokens = !line.tokens.is_empty();

                    if has_tokens && i + 1 < hunk.lines.len() && hunk.lines[i + 1].kind == LineKind::Modified {
                        let old_mod_line = &hunk.lines[i];
                        let new_mod_line = &hunk.lines[i + 1];

                        // Build left side with old_tokens
                        let old_line = build_token_line_old(old_mod_line);
                        // Build right side with new_tokens
                        let new_line = build_token_line_new(new_mod_line);

                        old_lines.push(old_line);
                        new_lines.push(new_line);
                        i += 2;
                    } else {
                        // Fallback: no tokens or unpaired Modified line
                        let old_line_no = format_line_no(line.old_line_no);
                        let old_text = line.old_text.as_deref().unwrap_or("");
                        let new_line_no = format_line_no(line.new_line_no);
                        let new_text = line.new_text.as_deref().unwrap_or("");

                        old_lines.push(Line::from(vec![
                            Span::styled(old_line_no, Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                old_text.to_string(),
                                Style::default().fg(Color::Red),
                            ),
                        ]));
                        new_lines.push(Line::from(vec![
                            Span::styled(new_line_no, Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                new_text.to_string(),
                                Style::default().fg(Color::Green),
                            ),
                        ]));
                        i += 1;
                    }
                }
            }
        }
    }

    (old_lines, new_lines)
}

/// Build the left (old) side line for a Modified line with token-level highlighting.
fn build_token_line_old(line: &DiffLine) -> Line<'static> {
    let line_no = format_line_no(line.old_line_no);
    let mut spans = vec![Span::styled(line_no, Style::default().fg(Color::DarkGray))];

    for token in &line.tokens {
        let style = match token.kind {
            ChangeKind::Equal => {
                // Subtle background to show the line is changed
                Style::default().bg(Color::Rgb(40, 0, 0))
            }
            ChangeKind::Deletion => {
                // Bright red fg + darker red bg + BOLD
                Style::default()
                    .fg(Color::Red)
                    .bg(Color::Rgb(80, 0, 0))
                    .add_modifier(Modifier::BOLD)
            }
            ChangeKind::Rename => {
                // Blue fg + blue bg + BOLD
                Style::default()
                    .fg(Color::Blue)
                    .bg(Color::Rgb(0, 0, 80))
                    .add_modifier(Modifier::BOLD)
            }
            ChangeKind::Addition => {
                // Addition tokens shouldn't appear on the old side,
                // but handle gracefully with dim styling
                Style::default().bg(Color::Rgb(40, 0, 0))
            }
        };
        spans.push(Span::styled(token.text.clone(), style));
    }

    Line::from(spans)
}

/// Build the right (new) side line for a Modified line with token-level highlighting.
fn build_token_line_new(line: &DiffLine) -> Line<'static> {
    let line_no = format_line_no(line.new_line_no);
    let mut spans = vec![Span::styled(line_no, Style::default().fg(Color::DarkGray))];

    for token in &line.tokens {
        let style = match token.kind {
            ChangeKind::Equal => {
                // Subtle background to show the line is changed
                Style::default().bg(Color::Rgb(0, 40, 0))
            }
            ChangeKind::Addition => {
                // Bright green fg + darker green bg + BOLD
                Style::default()
                    .fg(Color::Green)
                    .bg(Color::Rgb(0, 80, 0))
                    .add_modifier(Modifier::BOLD)
            }
            ChangeKind::Rename => {
                // Blue fg + blue bg + BOLD
                Style::default()
                    .fg(Color::Blue)
                    .bg(Color::Rgb(0, 0, 80))
                    .add_modifier(Modifier::BOLD)
            }
            ChangeKind::Deletion => {
                // Deletion tokens shouldn't appear on the new side,
                // but handle gracefully with dim styling
                Style::default().bg(Color::Rgb(0, 40, 0))
            }
        };
        spans.push(Span::styled(token.text.clone(), style));
    }

    Line::from(spans)
}

/// Format a line number as right-aligned in 4 characters, or spaces if None.
fn format_line_no(line_no: Option<usize>) -> String {
    match line_no {
        Some(n) => format!("{:>4} ", n),
        None => "     ".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::model::{
        ChangeKind, DiffLine, FileDiff, FileStatus, Hunk, LineKind, TokenChange,
    };
    use std::path::PathBuf;

    fn make_test_file() -> FileDiff {
        FileDiff {
            path: PathBuf::from("test.rs"),
            status: FileStatus::Modified,
            hunks: vec![Hunk {
                old_start: 1,
                new_start: 1,
                old_lines: 3,
                new_lines: 3,
                lines: vec![
                    // Context line
                    DiffLine {
                        kind: LineKind::Context,
                        old_line_no: Some(1),
                        new_line_no: Some(1),
                        old_text: Some("use std::io;".to_string()),
                        new_text: Some("use std::io;".to_string()),
                        tokens: vec![],
                    },
                    // Deleted line
                    DiffLine {
                        kind: LineKind::Deleted,
                        old_line_no: Some(2),
                        new_line_no: None,
                        old_text: Some("let x = 1;".to_string()),
                        new_text: None,
                        tokens: vec![],
                    },
                    // Added line
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

    fn make_modified_file() -> FileDiff {
        FileDiff {
            path: PathBuf::from("modified.rs"),
            status: FileStatus::Modified,
            hunks: vec![Hunk {
                old_start: 5,
                new_start: 5,
                old_lines: 1,
                new_lines: 1,
                lines: vec![
                    // Modified pair: old side tokens
                    DiffLine {
                        kind: LineKind::Modified,
                        old_line_no: Some(5),
                        new_line_no: Some(5),
                        old_text: Some("let x = 1;".to_string()),
                        new_text: Some("let x = 2;".to_string()),
                        tokens: vec![
                            TokenChange {
                                kind: ChangeKind::Equal,
                                text: "let x = ".to_string(),
                            },
                            TokenChange {
                                kind: ChangeKind::Deletion,
                                text: "1".to_string(),
                            },
                            TokenChange {
                                kind: ChangeKind::Equal,
                                text: ";".to_string(),
                            },
                        ],
                    },
                    // Modified pair: new side tokens
                    DiffLine {
                        kind: LineKind::Modified,
                        old_line_no: Some(5),
                        new_line_no: Some(5),
                        old_text: Some("let x = 1;".to_string()),
                        new_text: Some("let x = 2;".to_string()),
                        tokens: vec![
                            TokenChange {
                                kind: ChangeKind::Equal,
                                text: "let x = ".to_string(),
                            },
                            TokenChange {
                                kind: ChangeKind::Addition,
                                text: "2".to_string(),
                            },
                            TokenChange {
                                kind: ChangeKind::Equal,
                                text: ";".to_string(),
                            },
                        ],
                    },
                ],
            }],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }
    }

    #[test]
    fn test_build_side_by_side_basic() {
        let file = make_test_file();
        let (old, new) = build_side_by_side_lines(&file);

        // Should have: 1 hunk header + 1 context + 1 deleted + 1 added = 4 lines each
        assert_eq!(old.len(), 4, "Expected 4 old lines, got {}", old.len());
        assert_eq!(new.len(), 4, "Expected 4 new lines, got {}", new.len());
    }

    #[test]
    fn test_build_side_by_side_modified_pair() {
        let file = make_modified_file();
        let (old, new) = build_side_by_side_lines(&file);

        // Should have: 1 hunk header + 1 modified line = 2 lines each
        assert_eq!(old.len(), 2, "Expected 2 old lines, got {}", old.len());
        assert_eq!(new.len(), 2, "Expected 2 new lines, got {}", new.len());
    }

    #[test]
    fn test_format_line_no() {
        assert_eq!(format_line_no(Some(1)), "   1 ");
        assert_eq!(format_line_no(Some(42)), "  42 ");
        assert_eq!(format_line_no(Some(1234)), "1234 ");
        assert_eq!(format_line_no(None), "     ");
    }

    #[test]
    fn test_empty_file() {
        let file = FileDiff {
            path: PathBuf::from("empty.rs"),
            status: FileStatus::Modified,
            hunks: vec![],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        };
        let (old, new) = build_side_by_side_lines(&file);
        assert!(old.is_empty());
        assert!(new.is_empty());
    }
}
