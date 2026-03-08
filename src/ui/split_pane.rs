use std::collections::HashMap;
use std::path::Path;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::diff::model::{
    ChangeKind, CollapseLevel, DiffLine, FileDiff, FoldRegion, LineKind, MoveMatch,
};
use crate::syntax::{HighlightSpan, highlight_rust};
use crate::ui::animation::AnimationState;
use super::minimap::Minimap;

/// Render a side-by-side diff view with the old file on the left and the new file on the right.
pub fn render_split_pane(
    frame: &mut Frame,
    area: Rect,
    file: &FileDiff,
    scroll_offset: usize,
    collapse_level: CollapseLevel,
    animation: Option<&AnimationState>,
) {
    let [left_area, right_area, minimap_area] = Layout::horizontal([
        Constraint::Percentage(49),
        Constraint::Percentage(49),
        Constraint::Length(2),
    ])
    .areas(area);

    let is_rust = file
        .path
        .extension()
        .is_some_and(|ext| ext == "rs");

    let old_highlights = if is_rust {
        highlight_rust(&file.old_content)
    } else {
        Vec::new()
    };
    let new_highlights = if is_rust {
        highlight_rust(&file.new_content)
    } else {
        Vec::new()
    };

    let (old_lines, new_lines, hunk_start_offsets) =
        build_side_by_side_lines(file, &old_highlights, &new_highlights, collapse_level);

    let total_lines = old_lines.len();
    let visible_height = left_area.height.saturating_sub(2) as usize; // subtract 2 for borders

    let mut old_visible: Vec<Line> = old_lines
        .into_iter()
        .skip(scroll_offset)
        .take(visible_height)
        .collect();

    let mut new_visible: Vec<Line> = new_lines
        .into_iter()
        .skip(scroll_offset)
        .take(visible_height)
        .collect();

    // Apply hunk flash animation if active
    if let Some(anim) = animation {
        let flash_intensity = ((1.0 - anim.progress()) * 40.0) as u8;
        if flash_intensity > 0 {
            // Find the first hunk that starts at or after scroll_offset
            let (hunk_start, hunk_end) = find_current_hunk_range(
                &hunk_start_offsets,
                total_lines,
                scroll_offset,
            );
            // Convert absolute line indices to visible-window-relative indices
            let vis_start = hunk_start.saturating_sub(scroll_offset);
            let vis_end = hunk_end.saturating_sub(scroll_offset).min(visible_height);

            for i in vis_start..vis_end {
                if i < old_visible.len() {
                    old_visible[i] = apply_flash_to_line(old_visible[i].clone(), flash_intensity);
                }
                if i < new_visible.len() {
                    new_visible[i] = apply_flash_to_line(new_visible[i].clone(), flash_intensity);
                }
            }
        }
    }

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

    let visible_height = left_area.height.saturating_sub(2) as usize;
    frame.render_widget(
        Minimap::new(file, scroll_offset, visible_height),
        minimap_area,
    );
}

/// Build parallel line lists for left (old) and right (new) panes from the file's hunks.
///
/// Returns `(old_lines, new_lines, hunk_start_offsets)` where `hunk_start_offsets[i]` is
/// the index into the output lines where hunk `i` begins (at its `@@` header line).
fn build_side_by_side_lines(
    file: &FileDiff,
    old_highlights: &[Vec<HighlightSpan>],
    new_highlights: &[Vec<HighlightSpan>],
    collapse_level: CollapseLevel,
) -> (Vec<Line<'static>>, Vec<Line<'static>>, Vec<usize>) {
    let mut old_lines: Vec<Line<'static>> = Vec::new();
    let mut new_lines: Vec<Line<'static>> = Vec::new();
    let mut hunk_start_offsets: Vec<usize> = Vec::new();

    let fold_style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);

    for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
        // Between hunks, insert a collapsed marker or fold label for the gap
        if hunk_idx > 0 && collapse_level != CollapseLevel::Expanded {
            let prev_hunk = &file.hunks[hunk_idx - 1];
            let prev_old_end = prev_hunk.old_start + prev_hunk.old_lines;
            let gap_lines = hunk.old_start.saturating_sub(prev_old_end);

            if gap_lines > 0 {
                let label = make_gap_label(
                    gap_lines,
                    prev_old_end,
                    hunk.old_start.saturating_sub(1),
                    collapse_level,
                    &file.fold_regions,
                );
                old_lines.push(Line::from(Span::styled(label.clone(), fold_style)));
                new_lines.push(Line::from(Span::styled(label, fold_style)));
            }
        }

        // Record the start offset for this hunk (at its header line)
        hunk_start_offsets.push(old_lines.len());

        // Hunk header line on both sides
        let header = format!(
            "@@ -{},{} +{},{} @@",
            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
        );
        let header_style = Style::default().fg(Color::DarkGray);
        old_lines.push(Line::from(Span::styled(header.clone(), header_style)));
        new_lines.push(Line::from(Span::styled(header, header_style)));

        // Collect context runs within the hunk for potential collapsing in Scoped mode
        let hunk_lines = build_hunk_lines(hunk, old_highlights, new_highlights);

        if collapse_level == CollapseLevel::Scoped {
            // In Scoped mode, collapse runs of context lines that fall entirely
            // within a fold region (and are not within 3 lines of a changed line).
            let collapsed = collapse_context_in_hunk(
                &hunk_lines,
                hunk,
                &file.fold_regions,
                fold_style,
            );
            for (old_line, new_line) in collapsed {
                old_lines.push(old_line);
                new_lines.push(new_line);
            }
        } else {
            for (old_line, new_line) in hunk_lines {
                old_lines.push(old_line);
                new_lines.push(new_line);
            }
        }
    }

    (old_lines, new_lines, hunk_start_offsets)
}

/// Build the rendered line pairs for a single hunk (without collapsing).
fn build_hunk_lines(
    hunk: &crate::diff::model::Hunk,
    old_highlights: &[Vec<HighlightSpan>],
    new_highlights: &[Vec<HighlightSpan>],
) -> Vec<(Line<'static>, Line<'static>)> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < hunk.lines.len() {
        let line = &hunk.lines[i];

        match line.kind {
            LineKind::Context => {
                let line_no = format_line_no(line.old_line_no);
                let text = line.old_text.as_deref().unwrap_or("");

                let mut old_spans = vec![
                    Span::styled(line_no, Style::default().fg(Color::DarkGray)),
                ];
                old_spans.extend(apply_syntax_highlights(
                    text,
                    line.old_line_no,
                    old_highlights,
                ));
                let old_line = Line::from(old_spans);

                let new_line_no = format_line_no(line.new_line_no);
                let mut new_spans = vec![
                    Span::styled(new_line_no, Style::default().fg(Color::DarkGray)),
                ];
                new_spans.extend(apply_syntax_highlights(
                    text,
                    line.new_line_no,
                    new_highlights,
                ));
                let new_line = Line::from(new_spans);

                result.push((old_line, new_line));
                i += 1;
            }

            LineKind::Added => {
                let old_line = Line::from(Span::raw(String::new()));

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

                result.push((old_line, new_line));
                i += 1;
            }

            LineKind::Deleted => {
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

                let new_line = Line::from(Span::raw(String::new()));
                result.push((old_line, new_line));
                i += 1;
            }

            LineKind::Modified => {
                let has_tokens = !line.tokens.is_empty();

                if has_tokens && i + 1 < hunk.lines.len() && hunk.lines[i + 1].kind == LineKind::Modified {
                    let old_mod_line = &hunk.lines[i];
                    let new_mod_line = &hunk.lines[i + 1];

                    let old_line = build_token_line_old(old_mod_line);
                    let new_line = build_token_line_new(new_mod_line);

                    result.push((old_line, new_line));
                    i += 2;
                } else {
                    let old_line_no = format_line_no(line.old_line_no);
                    let old_text = line.old_text.as_deref().unwrap_or("");
                    let new_line_no = format_line_no(line.new_line_no);
                    let new_text = line.new_text.as_deref().unwrap_or("");

                    let old_line = Line::from(vec![
                        Span::styled(old_line_no, Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            old_text.to_string(),
                            Style::default().fg(Color::Red),
                        ),
                    ]);
                    let new_line = Line::from(vec![
                        Span::styled(new_line_no, Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            new_text.to_string(),
                            Style::default().fg(Color::Green),
                        ),
                    ]);

                    result.push((old_line, new_line));
                    i += 1;
                }
            }
        }
    }

    result
}

/// Generate a label for the gap between two hunks.
///
/// In Scoped mode, if a fold region covers the gap, use its label.
/// In Tight mode, use a simple "N lines hidden" label.
fn make_gap_label(
    gap_lines: usize,
    gap_old_start: usize,
    gap_old_end: usize,
    collapse_level: CollapseLevel,
    fold_regions: &[FoldRegion],
) -> String {
    if collapse_level == CollapseLevel::Scoped {
        // Find the best (innermost) fold region that covers this gap.
        // Fold regions use 0-indexed line numbers; hunk positions are 1-indexed.
        let gap_start_0 = gap_old_start.saturating_sub(1);
        let gap_end_0 = gap_old_end.saturating_sub(1);

        let best = fold_regions
            .iter()
            .filter(|r| r.old_start <= gap_start_0 && r.old_end >= gap_end_0)
            .min_by_key(|r| r.old_end - r.old_start);

        if let Some(region) = best {
            return format!("┈┈┈┈ {} ┈┈┈┈", region.label);
        }
    }

    format!("┈┈┈┈ {} lines hidden ┈┈┈┈", gap_lines)
}

/// Collapse runs of context lines within a hunk that fall entirely inside a fold region.
///
/// Walks through the hunk's rendered line pairs. Context lines that are part of a
/// contiguous run contained by a single fold region get replaced with a single fold
/// label line. Context lines near changed lines (within 3 lines) are kept.
fn collapse_context_in_hunk(
    lines: &[(Line<'static>, Line<'static>)],
    hunk: &crate::diff::model::Hunk,
    fold_regions: &[FoldRegion],
    fold_style: Style,
) -> Vec<(Line<'static>, Line<'static>)> {
    if fold_regions.is_empty() {
        return lines.to_vec();
    }

    // Build a map from rendered-line index to the corresponding DiffLine index,
    // accounting for Modified pairs consuming two DiffLines but one rendered line.
    let line_kinds: Vec<(LineKind, Option<usize>)> = {
        let mut kinds = Vec::new();
        let mut i = 0;
        while i < hunk.lines.len() {
            let dl = &hunk.lines[i];
            match dl.kind {
                LineKind::Modified => {
                    let has_tokens = !dl.tokens.is_empty();
                    if has_tokens && i + 1 < hunk.lines.len() && hunk.lines[i + 1].kind == LineKind::Modified {
                        kinds.push((LineKind::Modified, dl.old_line_no));
                        i += 2;
                    } else {
                        kinds.push((LineKind::Modified, dl.old_line_no));
                        i += 1;
                    }
                }
                _ => {
                    kinds.push((dl.kind, dl.old_line_no));
                    i += 1;
                }
            }
        }
        kinds
    };

    // Determine which rendered lines are "changed" (non-context)
    let is_changed: Vec<bool> = line_kinds.iter().map(|(k, _)| *k != LineKind::Context).collect();

    // Mark context lines that are within 3 lines of a change as "near change"
    let len = is_changed.len();
    let mut keep = vec![false; len];
    for (idx, &changed) in is_changed.iter().enumerate() {
        if changed {
            keep[idx] = true;
            // Mark 3 before and 3 after
            for delta in 1..=3 {
                if idx >= delta {
                    keep[idx - delta] = true;
                }
                if idx + delta < len {
                    keep[idx + delta] = true;
                }
            }
        }
    }

    // For context lines not marked as "keep", check if they fall inside a fold region
    // and group consecutive ones for collapsing.
    let mut result: Vec<(Line<'static>, Line<'static>)> = Vec::new();
    let mut i = 0;

    while i < len {
        if keep[i] || is_changed[i] {
            result.push(lines[i].clone());
            i += 1;
        } else {
            // Start of a collapsible context run
            let run_start = i;
            while i < len && !keep[i] && !is_changed[i] {
                i += 1;
            }
            let run_end = i; // exclusive
            let run_count = run_end - run_start;

            // Get the old line numbers for this run to find a matching fold region
            let first_old_line = line_kinds[run_start].1;
            let last_old_line = line_kinds[run_end - 1].1;

            let label = if let (Some(first), Some(last)) = (first_old_line, last_old_line) {
                // Convert 1-indexed to 0-indexed for fold region comparison
                let start_0 = first.saturating_sub(1);
                let end_0 = last.saturating_sub(1);

                let best = fold_regions
                    .iter()
                    .filter(|r| r.old_start <= start_0 && r.old_end >= end_0)
                    .min_by_key(|r| r.old_end - r.old_start);

                if let Some(region) = best {
                    format!("┈┈┈┈ {} ┈┈┈┈", region.label)
                } else {
                    format!("┈┈┈┈ {} lines hidden ┈┈┈┈", run_count)
                }
            } else {
                format!("┈┈┈┈ {} lines hidden ┈┈┈┈", run_count)
            };

            let marker_old = Line::from(Span::styled(label.clone(), fold_style));
            let marker_new = Line::from(Span::styled(label, fold_style));
            result.push((marker_old, marker_new));
        }
    }

    result
}

/// Find the range of output lines belonging to the first hunk at or after `scroll_offset`.
///
/// Returns `(start, end)` as absolute indices into the output line list (end is exclusive).
fn find_current_hunk_range(
    hunk_start_offsets: &[usize],
    total_lines: usize,
    scroll_offset: usize,
) -> (usize, usize) {
    // Find the first hunk whose start offset >= scroll_offset,
    // or failing that, the last hunk whose start is before scroll_offset
    // (i.e., the hunk the user is currently scrolled into).
    let mut best_idx = None;
    for (i, &start) in hunk_start_offsets.iter().enumerate() {
        if start >= scroll_offset {
            best_idx = Some(i);
            break;
        }
    }
    // If no hunk starts at or after scroll_offset, pick the last hunk
    // (the user is scrolled into it).
    if best_idx.is_none() && !hunk_start_offsets.is_empty() {
        best_idx = Some(hunk_start_offsets.len() - 1);
    }

    match best_idx {
        Some(idx) => {
            let start = hunk_start_offsets[idx];
            let end = if idx + 1 < hunk_start_offsets.len() {
                hunk_start_offsets[idx + 1]
            } else {
                total_lines
            };
            (start, end)
        }
        None => (0, 0), // no hunks
    }
}

/// Apply a flash highlight to a single `Line` by brightening all RGB background colors.
///
/// Adds `intensity` to each RGB channel of any background color, clamped to 255.
/// Non-RGB background colors get a subtle grey background overlay.
fn apply_flash_to_line(line: Line<'static>, intensity: u8) -> Line<'static> {
    let new_spans: Vec<Span<'static>> = line
        .spans
        .into_iter()
        .map(|span| {
            let style = span.style;
            let new_bg = match style.bg {
                Some(Color::Rgb(r, g, b)) => Some(Color::Rgb(
                    r.saturating_add(intensity),
                    g.saturating_add(intensity),
                    b.saturating_add(intensity),
                )),
                Some(other) => Some(other), // keep non-RGB backgrounds as-is
                None => Some(Color::Rgb(intensity, intensity, intensity)),
            };
            let new_style = Style {
                bg: new_bg,
                ..style
            };
            Span::styled(span.content, new_style)
        })
        .collect();
    Line::from(new_spans)
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

/// Apply syntax highlight spans to a line of text, returning a Vec of styled Spans.
///
/// `line_no` is 1-indexed (as stored in DiffLine). The highlights vec is 0-indexed.
/// If no highlights are available for the line, falls back to plain text.
fn apply_syntax_highlights(
    text: &str,
    line_no: Option<usize>,
    highlights: &[Vec<HighlightSpan>],
) -> Vec<Span<'static>> {
    // Convert 1-indexed line number to 0-indexed for highlight lookup
    let line_spans = line_no
        .and_then(|n| n.checked_sub(1))
        .and_then(|idx| highlights.get(idx));

    match line_spans {
        Some(spans) if !spans.is_empty() => {
            let mut result = Vec::new();
            let mut pos: usize = 0;

            for span in spans {
                // Skip spans that are out of range for this text
                if span.start >= text.len() {
                    break;
                }

                // Emit unstyled text for the gap before this span
                if span.start > pos {
                    let end = span.start.min(text.len());
                    if let Some(s) = text.get(pos..end) {
                        result.push(Span::raw(s.to_string()));
                    }
                }

                // Emit the styled span
                let start = span.start.max(pos);
                let end = span.end.min(text.len());
                if start < end
                    && let Some(s) = text.get(start..end)
                {
                    result.push(Span::styled(s.to_string(), span.style));
                }

                pos = span.end.min(text.len());
            }

            // Emit any remaining unstyled text after the last span
            if pos < text.len()
                && let Some(s) = text.get(pos..)
            {
                result.push(Span::raw(s.to_string()));
            }

            // If we produced nothing (all spans out of range), fall back
            if result.is_empty() {
                vec![Span::raw(text.to_string())]
            } else {
                result
            }
        }
        _ => {
            // No highlights available — plain text
            vec![Span::raw(text.to_string())]
        }
    }
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
        ChangeKind, CollapseLevel, DiffLine, FileDiff, FileStatus, FoldKind, FoldRegion, Hunk,
        LineKind, TokenChange,
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
        let no_hl: Vec<Vec<HighlightSpan>> = Vec::new();
        let (old, new, _offsets) = build_side_by_side_lines(&file, &no_hl, &no_hl, CollapseLevel::Expanded);

        // Should have: 1 hunk header + 1 context + 1 deleted + 1 added = 4 lines each
        assert_eq!(old.len(), 4, "Expected 4 old lines, got {}", old.len());
        assert_eq!(new.len(), 4, "Expected 4 new lines, got {}", new.len());
    }

    #[test]
    fn test_build_side_by_side_modified_pair() {
        let file = make_modified_file();
        let no_hl: Vec<Vec<HighlightSpan>> = Vec::new();
        let (old, new, _offsets) = build_side_by_side_lines(&file, &no_hl, &no_hl, CollapseLevel::Expanded);

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
        let no_hl: Vec<Vec<HighlightSpan>> = Vec::new();
        let (old, new, _offsets) = build_side_by_side_lines(&file, &no_hl, &no_hl, CollapseLevel::Expanded);
        assert!(old.is_empty());
        assert!(new.is_empty());
    }

    /// Build a file with two hunks separated by a gap, for testing collapse behavior.
    fn make_two_hunk_file() -> FileDiff {
        FileDiff {
            path: PathBuf::from("two_hunks.rs"),
            status: FileStatus::Modified,
            hunks: vec![
                Hunk {
                    old_start: 1,
                    new_start: 1,
                    old_lines: 2,
                    new_lines: 2,
                    lines: vec![
                        DiffLine {
                            kind: LineKind::Context,
                            old_line_no: Some(1),
                            new_line_no: Some(1),
                            old_text: Some("fn main() {".to_string()),
                            new_text: Some("fn main() {".to_string()),
                            tokens: vec![],
                        },
                        DiffLine {
                            kind: LineKind::Deleted,
                            old_line_no: Some(2),
                            new_line_no: None,
                            old_text: Some("    let a = 1;".to_string()),
                            new_text: None,
                            tokens: vec![],
                        },
                    ],
                },
                Hunk {
                    old_start: 20,
                    new_start: 20,
                    old_lines: 2,
                    new_lines: 2,
                    lines: vec![
                        DiffLine {
                            kind: LineKind::Added,
                            old_line_no: None,
                            new_line_no: Some(20),
                            old_text: None,
                            new_text: Some("    let b = 2;".to_string()),
                            tokens: vec![],
                        },
                        DiffLine {
                            kind: LineKind::Context,
                            old_line_no: Some(21),
                            new_line_no: Some(21),
                            old_text: Some("}".to_string()),
                            new_text: Some("}".to_string()),
                            tokens: vec![],
                        },
                    ],
                },
            ],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![],
            move_matches: vec![],
        }
    }

    #[test]
    fn test_tight_mode_shows_gap_marker_between_hunks() {
        let file = make_two_hunk_file();
        let no_hl: Vec<Vec<HighlightSpan>> = Vec::new();
        let (old, new, _offsets) = build_side_by_side_lines(&file, &no_hl, &no_hl, CollapseLevel::Tight);

        // Gap between hunks: old_start(1) + old_lines(2) = 3, next hunk starts at 20
        // So gap = 20 - 3 = 17 lines hidden.
        // Expected lines: header1(1) + 2 lines + gap marker(1) + header2(1) + 2 lines = 7
        assert_eq!(old.len(), 7, "Tight: expected 7 old lines, got {}", old.len());
        assert_eq!(new.len(), 7, "Tight: expected 7 new lines, got {}", new.len());

        // The gap marker should be at index 3 (after header + 2 hunk lines)
        let gap_text = old[3].to_string();
        assert!(
            gap_text.contains("17 lines hidden"),
            "Expected gap marker with '17 lines hidden', got: {}",
            gap_text
        );
    }

    #[test]
    fn test_scoped_mode_shows_fold_label_for_gap() {
        let mut file = make_two_hunk_file();
        // Add a fold region covering the gap (lines 2-19 in 0-indexed = lines 3-20 in 1-indexed)
        file.fold_regions.push(FoldRegion {
            kind: FoldKind::Function,
            label: "fn setup_database() (18 lines)".to_string(),
            old_start: 2, // 0-indexed
            old_end: 19,  // 0-indexed
            new_start: 2,
            new_end: 19,
            is_collapsed: false,
        });

        let no_hl: Vec<Vec<HighlightSpan>> = Vec::new();
        let (old, _new, _offsets) = build_side_by_side_lines(&file, &no_hl, &no_hl, CollapseLevel::Scoped);

        // The gap marker should use the fold label
        let gap_text = old[3].to_string();
        assert!(
            gap_text.contains("fn setup_database()"),
            "Expected fold label in gap marker, got: {}",
            gap_text
        );
    }

    #[test]
    fn test_expanded_mode_no_gap_markers() {
        let file = make_two_hunk_file();
        let no_hl: Vec<Vec<HighlightSpan>> = Vec::new();
        let (old, new, _offsets) = build_side_by_side_lines(&file, &no_hl, &no_hl, CollapseLevel::Expanded);

        // Expanded: no gap markers — just header + lines for each hunk
        // hunk1: header + 2 lines = 3, hunk2: header + 2 lines = 3, total = 6
        // No gap marker
        assert_eq!(old.len(), 6, "Expanded: expected 6 old lines, got {}", old.len());
        assert_eq!(new.len(), 6);

        // Verify none of the lines contain "hidden"
        for line in &old {
            let text = line.to_string();
            assert!(
                !text.contains("lines hidden"),
                "Expanded mode should not have gap markers, but found: {}",
                text
            );
        }
    }

    #[test]
    fn test_scoped_mode_collapses_context_within_fold_region() {
        // Build a hunk with many context lines inside a fold region,
        // with a changed line in the middle.
        let mut lines = Vec::new();
        // 5 context lines before the change (lines 10-14)
        for i in 10..15 {
            lines.push(DiffLine {
                kind: LineKind::Context,
                old_line_no: Some(i),
                new_line_no: Some(i),
                old_text: Some(format!("    line {};", i)),
                new_text: Some(format!("    line {};", i)),
                tokens: vec![],
            });
        }
        // Changed line at 15
        lines.push(DiffLine {
            kind: LineKind::Deleted,
            old_line_no: Some(15),
            new_line_no: None,
            old_text: Some("    old_line;".to_string()),
            new_text: None,
            tokens: vec![],
        });
        // 5 context lines after the change (lines 16-20)
        for i in 16..21 {
            lines.push(DiffLine {
                kind: LineKind::Context,
                old_line_no: Some(i),
                new_line_no: Some(i),
                old_text: Some(format!("    line {};", i)),
                new_text: Some(format!("    line {};", i)),
                tokens: vec![],
            });
        }

        let file = FileDiff {
            path: PathBuf::from("scoped.rs"),
            status: FileStatus::Modified,
            hunks: vec![Hunk {
                old_start: 10,
                new_start: 10,
                old_lines: 11,
                new_lines: 10,
                lines,
            }],
            old_content: String::new(),
            new_content: String::new(),
            fold_regions: vec![FoldRegion {
                kind: FoldKind::Function,
                label: "fn big_function() (30 lines)".to_string(),
                old_start: 5,  // 0-indexed, covers lines 6-35
                old_end: 34,
                new_start: 5,
                new_end: 34,
                is_collapsed: false,
            }],
            move_matches: vec![],
        };

        let no_hl: Vec<Vec<HighlightSpan>> = Vec::new();
        let (old, _new, _offsets) = build_side_by_side_lines(&file, &no_hl, &no_hl, CollapseLevel::Scoped);

        // In Expanded mode this would be: 1 header + 11 lines = 12
        let (old_expanded, _, _) =
            build_side_by_side_lines(&file, &no_hl, &no_hl, CollapseLevel::Expanded);
        assert_eq!(old_expanded.len(), 12);

        // In Scoped mode, context lines far from the change should be collapsed.
        // Lines 10 (idx 0) is 5 away from change at idx 5 -> collapsed
        // Lines 11 (idx 1) is 4 away from change at idx 5 -> collapsed
        // Lines 12-14 (idx 2-4) are within 3 of change at idx 5 -> kept
        // Change at idx 5 -> kept
        // Lines 16-18 (idx 6-8) are within 3 of change at idx 5 -> kept
        // Lines 19 (idx 9) is 4 away from change -> collapsed
        // Lines 20 (idx 10) is 5 away from change -> collapsed
        // So: 2 collapsed at start -> 1 marker, 3 kept, 1 change, 3 kept, 2 collapsed at end -> 1 marker
        // Total: header + 1 marker + 3 + 1 + 3 + 1 marker = 10
        assert!(
            old.len() < old_expanded.len(),
            "Scoped should have fewer lines ({}) than expanded ({})",
            old.len(),
            old_expanded.len()
        );

        // Should be exactly header(1) + fold_marker(1) + context(3) + change(1) + context(3) + fold_marker(1) = 10
        assert_eq!(old.len(), 10, "Expected 10 lines in scoped mode, got {}", old.len());
    }

    #[test]
    fn test_make_gap_label_tight() {
        let label = make_gap_label(15, 5, 19, CollapseLevel::Tight, &[]);
        assert!(label.contains("15 lines hidden"), "Got: {}", label);
    }

    #[test]
    fn test_make_gap_label_scoped_with_fold() {
        let regions = vec![FoldRegion {
            kind: FoldKind::Function,
            label: "fn foo() (20 lines)".to_string(),
            old_start: 3,  // 0-indexed
            old_end: 22,
            new_start: 3,
            new_end: 22,
            is_collapsed: false,
        }];
        // gap_old_start=5 (1-indexed), gap_old_end=19 (1-indexed)
        // -> gap_start_0=4, gap_end_0=18 -- both inside [3..22]
        let label = make_gap_label(15, 5, 19, CollapseLevel::Scoped, &regions);
        assert!(
            label.contains("fn foo()"),
            "Expected fold label, got: {}",
            label
        );
    }

    #[test]
    fn test_make_gap_label_scoped_without_fold() {
        let label = make_gap_label(15, 5, 19, CollapseLevel::Scoped, &[]);
        assert!(
            label.contains("15 lines hidden"),
            "Expected fallback label, got: {}",
            label
        );
    }
}
