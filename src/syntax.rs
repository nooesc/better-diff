use std::path::Path;
use std::sync::OnceLock;

use ratatui::style::Style;
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
    ["break" "case" "catch" "class" "const" "continue" "debugger" "default" "delete" "do" "else" "export" "extends" "finally" "for" "function" "if" "import" "in" "instanceof" "let" "new" "of" "return" "static" "switch" "throw" "try" "typeof" "var" "void" "while" "with" "yield" "async" "await"] @keyword
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
    ["break" "case" "catch" "class" "const" "continue" "debugger" "default" "delete" "do" "else" "export" "extends" "finally" "for" "function" "if" "import" "in" "instanceof" "let" "new" "of" "return" "static" "switch" "throw" "try" "typeof" "var" "void" "while" "with" "yield" "async" "await" "abstract" "declare" "enum" "implements" "interface" "keyof" "namespace" "private" "protected" "public" "readonly" "type" "override"] @keyword
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
    ["function" "end" "local" "if" "then" "else" "elseif" "for" "while" "do" "repeat" "until" "return" "in" "goto"] @keyword
    (function_declaration name: (identifier) @function)
    (function_call name: (identifier) @function_call)
"#;

const GO_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (interpreted_string_literal) @string
    (raw_string_literal) @string
    (rune_literal) @string
    (int_literal) @number
    (float_literal) @number
    (imaginary_literal) @number
    (true) @boolean
    (false) @boolean
    (nil) @boolean
    (type_identifier) @type
    ["func" "return" "if" "else" "for" "range" "switch" "case" "default" "break" "continue" "go" "defer" "select" "chan" "map" "struct" "interface" "package" "import" "var" "const" "type" "fallthrough" "goto"] @keyword
    (function_declaration name: (identifier) @function)
    (method_declaration name: (field_identifier) @function)
    (call_expression function: (identifier) @function_call)
    (call_expression function: (selector_expression field: (field_identifier) @function_call))
"#;

const C_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string_literal) @string
    (char_literal) @string
    (number_literal) @number
    (true) @boolean
    (false) @boolean
    (null) @boolean
    (type_identifier) @type
    (primitive_type) @type
    ["if" "else" "switch" "case" "default" "while" "do" "for" "return" "break" "continue" "goto" "typedef" "struct" "union" "enum" "extern" "static" "const" "sizeof"] @keyword
    (function_declarator declarator: (identifier) @function)
    (call_expression function: (identifier) @function_call)
"#;

const CPP_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string_literal) @string
    (raw_string_literal) @string
    (char_literal) @string
    (number_literal) @number
    (true) @boolean
    (false) @boolean
    (null) @boolean
    (type_identifier) @type
    (primitive_type) @type
    ["if" "else" "switch" "case" "default" "while" "do" "for" "return" "break" "continue" "goto" "typedef" "struct" "union" "enum" "extern" "static" "const" "sizeof" "class" "public" "private" "protected" "virtual" "override" "template" "typename" "namespace" "using" "new" "delete" "try" "catch" "throw" "constexpr" "inline"] @keyword
    (function_declarator declarator: (identifier) @function)
    (call_expression function: (identifier) @function_call)
"#;

const BASH_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string) @string
    (raw_string) @string
    (heredoc_body) @string
    ["if" "then" "else" "elif" "fi" "case" "esac" "for" "while" "until" "do" "done" "in" "function" "local" "declare" "export" "unset"] @keyword
    (function_definition name: (word) @function)
    (command_name (word) @function_call)
"#;

const JSON_QUERY_SOURCE: &str = r#"
    (string) @string
    (number) @number
    (true) @boolean
    (false) @boolean
    (null) @boolean
"#;

const TOML_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string) @string
    (integer) @number
    (float) @number
    (boolean) @boolean
    (bare_key) @keyword
    (table (bare_key) @type)
    (table_array_element (bare_key) @type)
"#;

const HTML_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (tag_name) @keyword
    (attribute_name) @type
    (quoted_attribute_value) @string
    (doctype) @macro
"#;

const CSS_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string_value) @string
    (color_value) @number
    (integer_value) @number
    (float_value) @number
    (property_name) @keyword
    (tag_name) @type
    (class_name) @type
    (id_name) @type
    (function_name) @function_call
"#;

const JAVA_QUERY_SOURCE: &str = r#"
    (line_comment) @comment
    (block_comment) @comment
    (string_literal) @string
    (character_literal) @string
    (decimal_integer_literal) @number
    (hex_integer_literal) @number
    (decimal_floating_point_literal) @number
    (true) @boolean
    (false) @boolean
    (null_literal) @boolean
    (type_identifier) @type
    ["abstract" "assert" "break" "case" "catch" "class" "continue" "default" "do" "else" "enum" "extends" "final" "finally" "for" "if" "implements" "import" "instanceof" "interface" "new" "package" "private" "protected" "public" "return" "static" "switch" "synchronized" "throw" "throws" "try" "volatile" "while"] @keyword
    (method_declaration name: (identifier) @function)
    (method_invocation name: (identifier) @function_call)
"#;

const RUBY_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string) @string
    (regex) @string
    (simple_symbol) @string
    (integer) @number
    (float) @number
    (true) @boolean
    (false) @boolean
    (nil) @boolean
    ["def" "end" "class" "module" "if" "else" "elsif" "unless" "while" "until" "for" "do" "begin" "rescue" "ensure" "return" "yield" "break" "next" "case" "when" "then" "in" "and" "or" "not"] @keyword
    (method name: (identifier) @function)
    (call method: (identifier) @function_call)
"#;

const CSHARP_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string_literal) @string
    (verbatim_string_literal) @string
    (character_literal) @string
    (integer_literal) @number
    (real_literal) @number
    (boolean_literal) @boolean
    (null_literal) @boolean
    (predefined_type) @type
    (generic_name (identifier) @type)
    ["abstract" "as" "base" "break" "case" "catch" "class" "const" "continue" "default" "do" "else" "enum" "event" "extern" "finally" "for" "foreach" "goto" "if" "in" "interface" "internal" "is" "namespace" "new" "operator" "out" "override" "params" "private" "protected" "public" "readonly" "ref" "return" "sealed" "sizeof" "static" "struct" "switch" "throw" "try" "typeof" "unsafe" "using" "virtual" "volatile" "while" "async" "await" "var"] @keyword
    (method_declaration name: (identifier) @function)
    (invocation_expression function: (identifier) @function_call)
"#;

const ELIXIR_QUERY_SOURCE: &str = r#"
    (comment) @comment
    (string) @string
    (integer) @number
    (float) @number
    (boolean) @boolean
    (nil) @boolean
    (atom) @string
    ["do" "end" "fn" "after" "rescue" "catch" "else" "when" "and" "or" "not" "in"] @keyword
    (call target: (identifier) @function_call)
"#;

const ZIG_QUERY_SOURCE: &str = r#"
    (comment) @comment
    ["fn" "return" "if" "else" "for" "while" "break" "continue" "switch" "const" "var" "pub" "extern" "struct" "enum" "union" "error" "test" "defer" "try" "catch" "unreachable" "comptime" "inline"] @keyword
"#;

/// Define a cached query function. Each invocation creates a function that lazily
/// compiles the tree-sitter highlight query once via `OnceLock`.
macro_rules! define_query {
    ($fn_name:ident, $ts_lang:expr, $query_source:expr) => {
        fn $fn_name() -> &'static Query {
            static QUERY: OnceLock<Query> = OnceLock::new();
            QUERY.get_or_init(|| {
                let language: tree_sitter::Language = $ts_lang.into();
                Query::new(&language, $query_source).expect(concat!(
                    "Failed to compile ",
                    stringify!($fn_name),
                    " highlight query"
                ))
            })
        }
    };
}

define_query!(rust_query, tree_sitter_rust::LANGUAGE, RUST_QUERY_SOURCE);
define_query!(js_query, tree_sitter_javascript::LANGUAGE, JS_QUERY_SOURCE);
define_query!(
    ts_query,
    tree_sitter_typescript::LANGUAGE_TSX,
    TS_QUERY_SOURCE
);
define_query!(
    python_query,
    tree_sitter_python::LANGUAGE,
    PYTHON_QUERY_SOURCE
);
define_query!(lua_query, tree_sitter_lua::LANGUAGE, LUA_QUERY_SOURCE);
define_query!(go_query, tree_sitter_go::LANGUAGE, GO_QUERY_SOURCE);
define_query!(c_query, tree_sitter_c::LANGUAGE, C_QUERY_SOURCE);
define_query!(cpp_query, tree_sitter_cpp::LANGUAGE, CPP_QUERY_SOURCE);
define_query!(bash_query, tree_sitter_bash::LANGUAGE, BASH_QUERY_SOURCE);
define_query!(json_query, tree_sitter_json::LANGUAGE, JSON_QUERY_SOURCE);
define_query!(toml_query, tree_sitter_toml_ng::LANGUAGE, TOML_QUERY_SOURCE);
define_query!(html_query, tree_sitter_html::LANGUAGE, HTML_QUERY_SOURCE);
define_query!(css_query, tree_sitter_css::LANGUAGE, CSS_QUERY_SOURCE);
define_query!(java_query, tree_sitter_java::LANGUAGE, JAVA_QUERY_SOURCE);
define_query!(ruby_query, tree_sitter_ruby::LANGUAGE, RUBY_QUERY_SOURCE);
define_query!(
    csharp_query,
    tree_sitter_c_sharp::LANGUAGE,
    CSHARP_QUERY_SOURCE
);
define_query!(
    elixir_query,
    tree_sitter_elixir::LANGUAGE,
    ELIXIR_QUERY_SOURCE
);
define_query!(zig_query, tree_sitter_zig::LANGUAGE, ZIG_QUERY_SOURCE);

#[derive(Debug, Clone, Copy)]
enum HighlightLanguage {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Lua,
    Go,
    C,
    Cpp,
    Bash,
    Json,
    Toml,
    Html,
    Css,
    Java,
    Ruby,
    CSharp,
    Elixir,
    Zig,
}

fn language_for_path(path: &Path) -> Option<HighlightLanguage> {
    // Also check full filename for extensionless files like Dockerfile, Makefile
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match filename {
        "Dockerfile" | "Containerfile" => return Some(HighlightLanguage::Bash),
        "Makefile" | "GNUmakefile" => return Some(HighlightLanguage::Bash),
        _ => {}
    }
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => Some(HighlightLanguage::Rust),
        Some("js" | "jsx" | "mjs" | "cjs") => Some(HighlightLanguage::JavaScript),
        Some("ts" | "tsx" | "mts" | "cts") => Some(HighlightLanguage::TypeScript),
        Some("py" | "pyi") => Some(HighlightLanguage::Python),
        Some("lua") => Some(HighlightLanguage::Lua),
        Some("go") => Some(HighlightLanguage::Go),
        Some("c" | "h") => Some(HighlightLanguage::C),
        Some("cc" | "cpp" | "cxx" | "hpp" | "hxx" | "hh") => Some(HighlightLanguage::Cpp),
        Some("sh" | "bash" | "zsh") => Some(HighlightLanguage::Bash),
        Some("json" | "jsonc") => Some(HighlightLanguage::Json),
        Some("toml") => Some(HighlightLanguage::Toml),
        Some("html" | "htm") => Some(HighlightLanguage::Html),
        Some("css" | "scss") => Some(HighlightLanguage::Css),
        Some("java") => Some(HighlightLanguage::Java),
        Some("rb" | "rake" | "gemspec") => Some(HighlightLanguage::Ruby),
        Some("cs") => Some(HighlightLanguage::CSharp),
        Some("ex" | "exs") => Some(HighlightLanguage::Elixir),
        Some("zig") => Some(HighlightLanguage::Zig),
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
        HighlightLanguage::Go => tree_sitter_go::LANGUAGE.into(),
        HighlightLanguage::C => tree_sitter_c::LANGUAGE.into(),
        HighlightLanguage::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        HighlightLanguage::Bash => tree_sitter_bash::LANGUAGE.into(),
        HighlightLanguage::Json => tree_sitter_json::LANGUAGE.into(),
        HighlightLanguage::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        HighlightLanguage::Html => tree_sitter_html::LANGUAGE.into(),
        HighlightLanguage::Css => tree_sitter_css::LANGUAGE.into(),
        HighlightLanguage::Java => tree_sitter_java::LANGUAGE.into(),
        HighlightLanguage::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        HighlightLanguage::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        HighlightLanguage::Elixir => tree_sitter_elixir::LANGUAGE.into(),
        HighlightLanguage::Zig => tree_sitter_zig::LANGUAGE.into(),
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
        HighlightLanguage::Go => go_query(),
        HighlightLanguage::C => c_query(),
        HighlightLanguage::Cpp => cpp_query(),
        HighlightLanguage::Bash => bash_query(),
        HighlightLanguage::Json => json_query(),
        HighlightLanguage::Toml => toml_query(),
        HighlightLanguage::Html => html_query(),
        HighlightLanguage::Css => css_query(),
        HighlightLanguage::Java => java_query(),
        HighlightLanguage::Ruby => ruby_query(),
        HighlightLanguage::CSharp => csharp_query(),
        HighlightLanguage::Elixir => elixir_query(),
        HighlightLanguage::Zig => zig_query(),
    }
}

/// Map a capture name to a syntax highlighting style.
fn style_for_capture(name: &str) -> Option<Style> {
    let t = crate::theme::current();
    match name {
        "comment" => Some(Style::default().fg(t.syntax_comment)),
        "string" => Some(Style::default().fg(t.syntax_string)),
        "number" | "boolean" => Some(Style::default().fg(t.syntax_number)),
        "type" => Some(Style::default().fg(t.syntax_type)),
        "keyword" => Some(Style::default().fg(t.syntax_keyword)),
        "function" | "function_call" => Some(Style::default().fg(t.syntax_function)),
        "macro" => Some(Style::default().fg(t.syntax_macro)),
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

    highlight_from_tree(tree, source, rust_query())
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
                let end_line =
                    line_for_offset(&line_starts, end_byte.saturating_sub(1).max(start_byte));

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
    use ratatui::style::Color;

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
        let fn_span = highlights[0].iter().find(|s| s.start == 0 && s.end == 2);
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

    #[test]
    fn test_all_language_queries_compile() {
        let test_cases: Vec<(&str, &str)> = vec![
            ("test.rs", "fn main() {}"),
            ("test.js", "function foo() {}"),
            ("test.ts", "function foo(): void {}"),
            ("test.py", "def foo(): pass"),
            ("test.lua", "function foo() end"),
            ("test.go", "func main() {}"),
            ("test.c", "int main() { return 0; }"),
            ("test.cpp", "int main() { return 0; }"),
            ("test.sh", "#!/bin/bash\necho hello"),
            ("test.json", r#"{"key": "value"}"#),
            ("test.toml", r#"[section]\nkey = "value""#),
            ("test.html", "<html><body></body></html>"),
            ("test.css", "body { color: red; }"),
            ("test.java", "class Main { void foo() {} }"),
            ("test.rb", "def foo; end"),
            ("test.cs", "class Main { void Foo() {} }"),
            ("test.ex", "defmodule Foo do end"),
            ("test.zig", "fn main() void {}"),
        ];

        let mut failures = Vec::new();
        for (filename, source) in test_cases {
            let result = std::panic::catch_unwind(|| {
                let path = Path::new(filename);
                highlight_file(path, source)
            });
            match result {
                Ok(highlights) => assert!(!highlights.is_empty()),
                Err(e) => {
                    let msg = e
                        .downcast_ref::<String>()
                        .map(|s| s.as_str())
                        .or_else(|| e.downcast_ref::<&str>().copied())
                        .unwrap_or("unknown panic");
                    failures.push(format!("{}: {}", filename, msg));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "Query compilation failures:\n{}",
            failures.join("\n")
        );
    }
}
