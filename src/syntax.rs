use std::sync::OnceLock;
use std::path::Path;

use ratatui::style::{Color, Style};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

#[derive(Debug, Clone)]
pub struct HighlightSpan {
    pub start: usize, // byte offset within line
    pub end: usize,   // byte offset within line
    pub style: Style,
}

const RUST_QUERY_SOURCE: &str = r#"
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

const JS_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string) @string
    (template_string) @string
    (regex) @string
    (number) @number
    (true) @boolean
    (false) @boolean
    ["break" "case" "catch" "class" "const" "continue" "debugger" "default" "delete" "do" "else" "export" "extends" "finally" "for" "function" "if" "import" "in" "instanceof" "let" "new" "of" "return" "static" "super" "switch" "this" "throw" "try" "typeof" "var" "void" "while" "with" "yield" "async" "await"] @keyword
    (function_declaration name: (identifier) @function)
    (generator_function_declaration name: (identifier) @function)
    (method_definition name: (property_identifier) @function)
    (call_expression function: (identifier) @function_call)
    (call_expression function: (member_expression property: (property_identifier) @function_call))
    (jsx_opening_element name: (identifier) @type)
    (jsx_closing_element name: (identifier) @type)
    (jsx_self_closing_element name: (identifier) @type)
"#;

const TS_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string) @string
    (template_string) @string
    (regex) @string
    (number) @number
    (true) @boolean
    (false) @boolean
    (type_identifier) @type
    (predefined_type) @type
    ["break" "case" "catch" "class" "const" "continue" "debugger" "default" "delete" "do" "else" "export" "extends" "finally" "for" "function" "if" "import" "in" "instanceof" "let" "new" "of" "return" "static" "super" "switch" "this" "throw" "try" "typeof" "var" "void" "while" "with" "yield" "async" "await" "abstract" "declare" "enum" "implements" "interface" "keyof" "namespace" "private" "protected" "public" "readonly" "type" "override"] @keyword
    (function_declaration name: (identifier) @function)
    (generator_function_declaration name: (identifier) @function)
    (method_definition name: (property_identifier) @function)
    (call_expression function: (identifier) @function_call)
    (call_expression function: (member_expression property: (property_identifier) @function_call))
    (jsx_opening_element name: (identifier) @type)
    (jsx_closing_element name: (identifier) @type)
    (jsx_self_closing_element name: (identifier) @type)
"#;

const PYTHON_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string) @string
    (integer) @number
    (float) @number
    (true) @boolean
    (false) @boolean
    (none) @boolean
    ["and" "as" "assert" "async" "await" "break" "class" "continue" "def" "del" "elif" "else" "except" "finally" "for" "from" "global" "if" "import" "in" "is" "lambda" "nonlocal" "not" "or" "pass" "raise" "return" "try" "while" "with" "yield"] @keyword
    (function_definition name: (identifier) @function)
    (call function: (identifier) @function_call)
    (call function: (attribute attribute: (identifier) @function_call))
    (decorator) @macro
"#;

const LUA_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string) @string
    (number) @number
    ["function" "end" "local" "if" "then" "else" "elseif" "for" "while" "do" "repeat" "until" "return" "break" "in" "and" "or" "not" "goto" "nil" "true" "false"] @keyword
    (function_declaration name: (identifier) @function)
    (function_call name: (identifier) @function_call)
"#;

/// Define a cached query function. Each invocation creates a function that lazily
/// compiles the tree-sitter highlight query once via `OnceLock`.
macro_rules! define_query {
    ($fn_name:ident, $ts_lang:expr, $query_source:expr) => {
        fn $fn_name() -> &'static Query {
            static QUERY: OnceLock<Query> = OnceLock::new();
            QUERY.get_or_init(|| {
                let language: tree_sitter::Language = $ts_lang.into();
                Query::new(&language, $query_source)
                    .expect(concat!("Failed to compile ", stringify!($fn_name), " highlight query"))
            })
        }
    };
}

define_query!(rust_query, tree_sitter_rust::LANGUAGE, RUST_QUERY_SOURCE);
define_query!(js_query, tree_sitter_javascript::LANGUAGE, JS_QUERY_SOURCE);
define_query!(ts_query, tree_sitter_typescript::LANGUAGE_TSX, TS_QUERY_SOURCE);
define_query!(python_query, tree_sitter_python::LANGUAGE, PYTHON_QUERY_SOURCE);
define_query!(lua_query, tree_sitter_lua::LANGUAGE, LUA_QUERY_SOURCE);

#[derive(Debug, Clone, Copy)]
enum HighlightLanguage {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Lua,
}

fn language_for_path(path: &Path) -> Option<HighlightLanguage> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => Some(HighlightLanguage::Rust),
        Some("js" | "jsx" | "mjs" | "cjs") => Some(HighlightLanguage::JavaScript),
        Some("ts" | "tsx" | "mts" | "cts") => Some(HighlightLanguage::TypeScript),
        Some("py" | "pyi") => Some(HighlightLanguage::Python),
        Some("lua") => Some(HighlightLanguage::Lua),
        _ => None,
    }
}

/// Parse Rust source code using tree-sitter and return the syntax tree.
///
/// Returns `None` if parsing fails.
pub fn parse_rust(source: &str) -> Option<tree_sitter::Tree> {
    parse_language(HighlightLanguage::Rust, source)
}

fn parse_language(language: HighlightLanguage, source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    let ts_language = match language {
        HighlightLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
        HighlightLanguage::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        HighlightLanguage::TypeScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
        HighlightLanguage::Python => tree_sitter_python::LANGUAGE.into(),
        HighlightLanguage::Lua => tree_sitter_lua::LANGUAGE.into(),
    };

    if parser.set_language(&ts_language).is_err() {
        return None;
    }
    parser.parse(source, None)
}

fn syntax_query(language: HighlightLanguage) -> &'static Query {
    match language {
        HighlightLanguage::Rust => rust_query(),
        HighlightLanguage::JavaScript => js_query(),
        HighlightLanguage::TypeScript => ts_query(),
        HighlightLanguage::Python => python_query(),
        HighlightLanguage::Lua => lua_query(),
    }
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
    let Some(tree) = parse_rust(source) else {
        return vec![Vec::new(); build_line_starts(source).len()];
    };

    highlight_from_tree(tree, source, &rust_query())
}

/// Parse and highlight a source file by path-driven language detection.
/// Unrecognized extensions fall back to no highlighting.
pub fn highlight_file(path: &Path, source: &str) -> Vec<Vec<HighlightSpan>> {
    let Some(language) = language_for_path(path) else {
        return vec![Vec::new(); build_line_starts(source).len()];
    };

    let Some(tree) = parse_language(language, source) else {
        return vec![Vec::new(); build_line_starts(source).len()];
    };

    highlight_from_tree(tree, source, syntax_query(language))
}

fn highlight_from_tree(
    tree: tree_sitter::Tree,
    source: &str,
    query: &'static Query,
) -> Vec<Vec<HighlightSpan>> {
    let line_starts = build_line_starts(source);
    let num_lines = line_starts.len(); // accounts for trailing content after last \n
    let mut result: Vec<Vec<HighlightSpan>> = vec![Vec::new(); num_lines];
    
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
