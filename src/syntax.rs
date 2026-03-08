use std::sync::OnceLock;

use ratatui::style::{Color, Style};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

#[derive(Debug, Clone)]
pub struct HighlightSpan {
    pub start: usize, // byte offset within line
    pub end: usize,   // byte offset within line
    pub style: Style,
}

const QUERY_SOURCE: &str = r#"
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
    (mutable_specifier) @keyword
    (crate) @keyword
    (super) @keyword
    ["fn" "let" "pub" "use" "mod" "struct" "enum" "impl" "trait" "for" "while" "loop" "if" "else" "match" "return" "async" "await" "move" "ref" "where" "type" "const" "static" "unsafe" "extern" "as" "in" "break" "continue" "dyn"] @keyword
    (function_item name: (identifier) @function)
    (call_expression function: (identifier) @function_call)
    (macro_invocation macro: (identifier) @macro)
"#;

fn rust_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    QUERY.get_or_init(|| {
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        Query::new(&language, QUERY_SOURCE).expect("Failed to compile highlight query")
    })
}

/// Parse Rust source code using tree-sitter and return the syntax tree.
///
/// Returns `None` if parsing fails.
pub fn parse_rust(source: &str) -> Option<tree_sitter::Tree> {
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return None;
    }
    parser.parse(source, None)
}

/// Map a capture name to a syntax highlighting style.
fn style_for_capture(name: &str) -> Option<Style> {
    match name {
        "comment" => Some(Style::default().fg(Color::DarkGray)),
        "string" => Some(Style::default().fg(Color::Rgb(206, 145, 120))),
        "number" | "boolean" => Some(Style::default().fg(Color::Rgb(181, 206, 168))),
        "type" => Some(Style::default().fg(Color::Rgb(78, 201, 176))),
        "keyword" => Some(Style::default().fg(Color::Rgb(197, 134, 192))),
        "function" | "function_call" => Some(Style::default().fg(Color::Rgb(220, 220, 170))),
        "macro" => Some(Style::default().fg(Color::Rgb(86, 156, 214))),
        _ => None,
    }
}

/// Build a table mapping each byte offset to its line number (0-indexed).
/// Returns (line_starts, total_lines) where line_starts[i] is the byte offset
/// where line i begins.
fn build_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, ch) in source.bytes().enumerate() {
        if ch == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Given a byte offset and sorted line_starts, return the 0-indexed line number.
fn line_for_offset(line_starts: &[usize], offset: usize) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(line) => line,
        Err(line) => line.saturating_sub(1),
    }
}

/// Parse Rust source and return highlight spans per line (0-indexed).
///
/// The returned Vec has one entry per line in the source. Each entry contains
/// the highlight spans for that line, sorted by start offset within the line.
pub fn highlight_rust(source: &str) -> Vec<Vec<HighlightSpan>> {
    let line_starts = build_line_starts(source);
    let num_lines = line_starts.len(); // accounts for trailing content after last \n
    let mut result: Vec<Vec<HighlightSpan>> = vec![Vec::new(); num_lines];

    let tree = match parse_rust(source) {
        Some(tree) => tree,
        None => return result,
    };

    let query = rust_query();
    let capture_names = query.capture_names();
    let root = tree.root_node();

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source.as_bytes());

    while let Some(m) = {
        matches.advance();
        matches.get()
    } {
        for capture in m.captures {
            let name = capture_names[capture.index as usize];
            if let Some(style) = style_for_capture(name) {
                let node = capture.node;
                let start_byte = node.start_byte();
                let end_byte = node.end_byte();
                let start_line = line_for_offset(&line_starts, start_byte);
                let end_line = line_for_offset(&line_starts, end_byte.saturating_sub(1).max(start_byte));

                for line in start_line..=end_line {
                    if line >= num_lines {
                        break;
                    }
                    let line_start = line_starts[line];
                    let line_end = if line + 1 < line_starts.len() {
                        line_starts[line + 1]
                    } else {
                        source.len()
                    };

                    let span_start = start_byte.max(line_start) - line_start;
                    let span_end = end_byte.min(line_end) - line_start;

                    if span_start < span_end {
                        result[line].push(HighlightSpan {
                            start: span_start,
                            end: span_end,
                            style,
                        });
                    }
                }
            }
        }
    }

    // Sort spans within each line by start offset
    for line_spans in &mut result {
        line_spans.sort_by_key(|s| s.start);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_rust_basic() {
        let source = "fn main() {\n    let x = 42;\n}\n";
        let highlights = highlight_rust(source);

        // First line should have highlights (at least "fn" keyword and "main" function name)
        assert!(
            !highlights[0].is_empty(),
            "Expected highlights on the first line, got none"
        );

        // Verify "fn" keyword is highlighted
        let fn_span = highlights[0]
            .iter()
            .find(|s| s.start == 0 && s.end == 2);
        assert!(
            fn_span.is_some(),
            "Expected 'fn' keyword to be highlighted on line 0"
        );
    }

    #[test]
    fn test_highlight_empty_source() {
        let highlights = highlight_rust("");
        // Even an empty string should return at least 1 entry (for the single "line")
        assert!(
            !highlights.is_empty(),
            "Expected at least 1 entry for empty source"
        );
    }

    #[test]
    fn test_highlight_preserves_line_count() {
        let source = "let a = 1;\nlet b = 2;\nlet c = 3;\n";
        let highlights = highlight_rust(source);
        // Source has 3 lines of content + 1 trailing empty line = 4 line starts
        // But we count lines as line_starts.len() which is 4
        assert!(
            highlights.len() >= 3,
            "Expected at least 3 entries for 3-line source, got {}",
            highlights.len()
        );
    }

    #[test]
    fn test_highlight_comment() {
        let source = "// this is a comment\nlet x = 1;\n";
        let highlights = highlight_rust(source);

        // First line should have a comment highlight
        let comment_spans: Vec<_> = highlights[0]
            .iter()
            .filter(|s| s.style.fg == Some(Color::DarkGray))
            .collect();
        assert!(
            !comment_spans.is_empty(),
            "Expected comment highlight on first line"
        );
    }

    #[test]
    fn test_highlight_string_literal() {
        let source = "let s = \"hello\";\n";
        let highlights = highlight_rust(source);

        let string_color = Color::Rgb(206, 145, 120);
        let string_spans: Vec<_> = highlights[0]
            .iter()
            .filter(|s| s.style.fg == Some(string_color))
            .collect();
        assert!(
            !string_spans.is_empty(),
            "Expected string literal highlight"
        );
    }
}
