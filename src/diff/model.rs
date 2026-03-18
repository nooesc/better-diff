use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    WorkingTree,
    Staged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Deleted,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Equal,
    Rename,
    Addition,
    Deletion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenChange {
    pub kind: ChangeKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_line_no: Option<usize>,
    pub new_line_no: Option<usize>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub tokens: Vec<TokenChange>,
}

impl DiffLine {
    pub fn old_str(&self) -> &str {
        self.old_text.as_deref().unwrap_or("")
    }

    pub fn new_str(&self) -> &str {
        self.new_text.as_deref().unwrap_or("")
    }
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub old_start: usize,
    pub new_start: usize,
    pub old_lines: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldKind {
    Function,
    Impl,
    Module,
    Struct,
    Enum,
    Other,
}

#[derive(Debug, Clone)]
pub struct FoldRegion {
    pub kind: FoldKind,
    pub label: String,
    pub old_start: usize,
    pub old_end: usize,
}

#[derive(Debug, Clone)]
pub struct MoveMatch {
    pub source_file: PathBuf,
    pub source_start: usize,
    pub source_end: usize,
    pub dest_file: PathBuf,
    pub dest_start: usize,
    pub dest_end: usize,
    pub similarity: f64,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub status: FileStatus,
    pub hunks: Vec<Hunk>,
    pub old_content: String,
    pub new_content: String,
    pub fold_regions: Vec<FoldRegion>,
    pub move_matches: Vec<MoveMatch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapseLevel {
    Tight,
    Scoped,
    Expanded,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_line_context() {
        let line = DiffLine {
            kind: LineKind::Context,
            old_line_no: Some(10),
            new_line_no: Some(10),
            old_text: Some("let x = 1;".to_string()),
            new_text: Some("let x = 1;".to_string()),
            tokens: vec![TokenChange {
                kind: ChangeKind::Equal,
                text: "let x = 1;".to_string(),
            }],
        };

        assert_eq!(line.kind, LineKind::Context);
        assert_eq!(line.old_line_no, Some(10));
        assert_eq!(line.new_line_no, Some(10));
        assert_eq!(line.tokens.len(), 1);
        assert_eq!(line.tokens[0].kind, ChangeKind::Equal);
    }

    #[test]
    fn test_diff_line_modified_with_tokens() {
        let line = DiffLine {
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
                    kind: ChangeKind::Addition,
                    text: "2".to_string(),
                },
                TokenChange {
                    kind: ChangeKind::Equal,
                    text: ";".to_string(),
                },
            ],
        };

        assert_eq!(line.kind, LineKind::Modified);
        assert_eq!(line.tokens.len(), 4);
        assert_eq!(line.tokens[1].kind, ChangeKind::Deletion);
        assert_eq!(line.tokens[1].text, "1");
        assert_eq!(line.tokens[2].kind, ChangeKind::Addition);
        assert_eq!(line.tokens[2].text, "2");
    }

    #[test]
    fn test_file_diff_creation() {
        let diff = FileDiff {
            path: PathBuf::from("src/main.rs"),
            old_path: None,
            status: FileStatus::Modified,
            hunks: vec![Hunk {
                old_start: 1,
                new_start: 1,
                old_lines: 3,
                new_lines: 4,
                lines: vec![DiffLine {
                    kind: LineKind::Added,
                    old_line_no: None,
                    new_line_no: Some(2),
                    old_text: None,
                    new_text: Some("new line".to_string()),
                    tokens: vec![TokenChange {
                        kind: ChangeKind::Addition,
                        text: "new line".to_string(),
                    }],
                }],
            }],
            old_content: "line1\nline2\nline3\n".to_string(),
            new_content: "line1\nnew line\nline2\nline3\n".to_string(),
            fold_regions: vec![],
            move_matches: vec![],
        };

        assert_eq!(diff.path, PathBuf::from("src/main.rs"));
        assert_eq!(diff.status, FileStatus::Modified);
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].lines.len(), 1);
        assert_eq!(diff.hunks[0].old_lines, 3);
        assert_eq!(diff.hunks[0].new_lines, 4);
    }

    #[test]
    fn test_collapse_level_inequality() {
        assert_ne!(CollapseLevel::Tight, CollapseLevel::Scoped);
        assert_ne!(CollapseLevel::Scoped, CollapseLevel::Expanded);
        assert_ne!(CollapseLevel::Tight, CollapseLevel::Expanded);
        assert_eq!(CollapseLevel::Tight, CollapseLevel::Tight);
        assert_eq!(CollapseLevel::Scoped, CollapseLevel::Scoped);
        assert_eq!(CollapseLevel::Expanded, CollapseLevel::Expanded);
    }
}
