use tree_sitter::TreeCursor;

use super::model::{FoldKind, FoldRegion};
use crate::syntax::parse_rust;

/// Compute fold regions from Rust source code using tree-sitter AST analysis.
///
/// Walks the AST looking for foldable constructs (functions, impls, modules,
/// structs, enums) that span more than 3 lines. Returns a list of `FoldRegion`
/// values with descriptive labels.
pub fn compute_fold_regions(source: &str) -> Vec<FoldRegion> {
    let tree = match parse_rust(source) {
        Some(tree) => tree,
        None => return Vec::new(),
    };

    let mut regions = Vec::new();
    let mut cursor = tree.walk();
    collect_fold_regions(&mut cursor, source, &mut regions);
    regions
}

/// Map a tree-sitter node kind to a FoldKind, if it is foldable.
fn fold_kind_for_node(kind: &str) -> Option<FoldKind> {
    match kind {
        "function_item" => Some(FoldKind::Function),
        "impl_item" => Some(FoldKind::Impl),
        "mod_item" => Some(FoldKind::Module),
        "struct_item" => Some(FoldKind::Struct),
        "enum_item" => Some(FoldKind::Enum),
        _ => None,
    }
}

/// Build a human-readable label for a fold region.
fn build_label(fold_kind: &FoldKind, name: Option<&str>, line_count: usize) -> String {
    let kind_str = match fold_kind {
        FoldKind::Function => "fn",
        FoldKind::Impl => "impl",
        FoldKind::Module => "mod",
        FoldKind::Struct => "struct",
        FoldKind::Enum => "enum",
        FoldKind::Other => "block",
    };

    match name {
        Some(n) => format!("{kind_str} {n} ({line_count} lines)"),
        None => format!("{kind_str} ({line_count} lines)"),
    }
}

/// Recursively walk the AST, collecting fold regions for foldable nodes.
fn collect_fold_regions(cursor: &mut TreeCursor, source: &str, regions: &mut Vec<FoldRegion>) {
    loop {
        let node = cursor.node();
        let kind = node.kind();

        if let Some(fold_kind) = fold_kind_for_node(kind) {
            let start_line = node.start_position().row;
            let end_line = node.end_position().row;
            let line_count = end_line - start_line + 1;

            // Only fold regions spanning more than 3 lines
            if line_count > 3 {
                // Extract name from the "name" field if available
                let name = node
                    .child_by_field_name("name")
                    .map(|n| &source[n.start_byte()..n.end_byte()]);

                // For impl items, try to get the type name instead
                let name = if name.is_none() && fold_kind == FoldKind::Impl {
                    node.child_by_field_name("type")
                        .map(|n| &source[n.start_byte()..n.end_byte()])
                } else {
                    name
                };

                let label = build_label(&fold_kind, name, line_count);

                regions.push(FoldRegion {
                    kind: fold_kind,
                    label,
                    old_start: start_line,
                    old_end: end_line,
                });
            }
        }

        // Recurse into children
        if cursor.goto_first_child() {
            collect_fold_regions(cursor, source, regions);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fold_function() {
        let source = r#"fn hello() {
    let a = 1;
    let b = 2;
    let c = 3;
    let d = 4;
}
"#;
        let regions = compute_fold_regions(source);
        assert!(
            !regions.is_empty(),
            "Expected at least one fold region for a 6-line function"
        );

        let func_region = regions
            .iter()
            .find(|r| r.kind == FoldKind::Function)
            .expect("Expected a Function fold region");

        assert!(
            func_region.label.contains("hello"),
            "Expected label to contain function name 'hello', got: {}",
            func_region.label
        );
        assert!(
            func_region.label.contains("lines"),
            "Expected label to contain line count"
        );
    }

    #[test]
    fn test_fold_impl() {
        let source = r#"struct Foo;

impl Foo {
    fn bar(&self) {
        todo!()
    }

    fn baz(&self) {
        todo!()
    }
}
"#;
        let regions = compute_fold_regions(source);

        let impl_region = regions
            .iter()
            .find(|r| r.kind == FoldKind::Impl)
            .expect("Expected an Impl fold region");

        assert!(
            impl_region.label.contains("Foo"),
            "Expected impl label to contain type name 'Foo', got: {}",
            impl_region.label
        );
    }

    #[test]
    fn test_short_function_not_folded() {
        let source = r#"fn short() {
    1 + 1;
}
"#;
        let regions = compute_fold_regions(source);

        let func_regions: Vec<_> = regions
            .iter()
            .filter(|r| r.kind == FoldKind::Function)
            .collect();

        assert!(
            func_regions.is_empty(),
            "Expected no fold region for a 3-line function, but found {}",
            func_regions.len()
        );
    }

    #[test]
    fn test_empty_source() {
        let regions = compute_fold_regions("");
        assert!(
            regions.is_empty(),
            "Expected no fold regions for empty source"
        );
    }

    #[test]
    fn test_fold_struct() {
        let source = r#"struct MyStruct {
    field1: i32,
    field2: String,
    field3: bool,
    field4: Vec<u8>,
}
"#;
        let regions = compute_fold_regions(source);

        let struct_region = regions
            .iter()
            .find(|r| r.kind == FoldKind::Struct);

        assert!(
            struct_region.is_some(),
            "Expected a Struct fold region for a multi-line struct"
        );

        let struct_region = struct_region.unwrap();
        assert!(
            struct_region.label.contains("MyStruct"),
            "Expected struct label to contain 'MyStruct', got: {}",
            struct_region.label
        );
    }

    #[test]
    fn test_fold_enum() {
        let source = r#"enum Color {
    Red,
    Green,
    Blue,
    Custom(u8, u8, u8),
}
"#;
        let regions = compute_fold_regions(source);

        let enum_region = regions
            .iter()
            .find(|r| r.kind == FoldKind::Enum);

        assert!(
            enum_region.is_some(),
            "Expected an Enum fold region for a multi-line enum"
        );
    }
}
