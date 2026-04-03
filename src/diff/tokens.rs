use similar::{ChangeTag, TextDiff};

use super::model::{ChangeKind, TokenChange};

/// Compute word-level token changes between two strings.
/// Returns a tuple of (old_tokens, new_tokens).
pub fn compute_token_changes(old: &str, new: &str) -> (Vec<TokenChange>, Vec<TokenChange>) {
    let diff = TextDiff::from_words(old, new);
    let mut old_tokens = Vec::new();
    let mut new_tokens = Vec::new();

    for change in diff.iter_all_changes() {
        let text = change.value().to_string();
        match change.tag() {
            ChangeTag::Equal => {
                old_tokens.push(TokenChange {
                    kind: ChangeKind::Equal,
                    text: text.clone(),
                });
                new_tokens.push(TokenChange {
                    kind: ChangeKind::Equal,
                    text,
                });
            }
            ChangeTag::Delete => {
                old_tokens.push(TokenChange {
                    kind: ChangeKind::Deletion,
                    text,
                });
            }
            ChangeTag::Insert => {
                new_tokens.push(TokenChange {
                    kind: ChangeKind::Addition,
                    text,
                });
            }
        }
    }

    promote_renames(&mut old_tokens, &mut new_tokens);

    (old_tokens, new_tokens)
}

/// Find paired deletions and additions of non-whitespace tokens and promote
/// them to `ChangeKind::Rename`.
fn promote_renames(old_tokens: &mut [TokenChange], new_tokens: &mut [TokenChange]) {
    let mut deletions: Vec<usize> = old_tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == ChangeKind::Deletion && t.text.trim() != "")
        .map(|(i, _)| i)
        .collect();

    let mut additions: Vec<usize> = new_tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == ChangeKind::Addition && t.text.trim() != "")
        .map(|(i, _)| i)
        .collect();

    let pairs = deletions.len().min(additions.len());
    deletions.truncate(pairs);
    additions.truncate(pairs);

    for (del_idx, add_idx) in deletions.into_iter().zip(additions.into_iter()) {
        old_tokens[del_idx].kind = ChangeKind::Rename;
        new_tokens[add_idx].kind = ChangeKind::Rename;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_strings() {
        let (old, new) = compute_token_changes("hello world", "hello world");
        assert!(
            old.iter().all(|t| t.kind == ChangeKind::Equal),
            "All old tokens should be Equal"
        );
        assert!(
            new.iter().all(|t| t.kind == ChangeKind::Equal),
            "All new tokens should be Equal"
        );
    }

    #[test]
    fn test_rename_detection() {
        // Use space-separated tokens to get clean word boundaries
        let (old, new) = compute_token_changes("foo a b", "bar a b");
        // "foo" was deleted and "bar" was inserted — should be promoted to Rename
        let old_renames: Vec<_> = old
            .iter()
            .filter(|t| t.kind == ChangeKind::Rename)
            .collect();
        let new_renames: Vec<_> = new
            .iter()
            .filter(|t| t.kind == ChangeKind::Rename)
            .collect();

        assert_eq!(old_renames.len(), 1, "Expected one rename in old tokens");
        assert_eq!(old_renames[0].text, "foo");
        assert_eq!(new_renames.len(), 1, "Expected one rename in new tokens");
        assert_eq!(new_renames[0].text, "bar");
    }

    #[test]
    fn test_addition_detection() {
        let (old, new) = compute_token_changes("foo(a, b)", "foo(a, b, c)");
        // There should be additions in new_tokens for the extra ", c" part
        let additions: Vec<_> = new
            .iter()
            .filter(|t| t.kind == ChangeKind::Addition || t.kind == ChangeKind::Rename)
            .collect();

        assert!(!additions.is_empty(), "Expected addition tokens in new");

        // Old tokens should have no deletions (only equal or whitespace changes)
        let old_non_equal: Vec<_> = old
            .iter()
            .filter(|t| t.kind == ChangeKind::Deletion || t.kind == ChangeKind::Rename)
            .collect();

        // The diff may detect some tokens as changed depending on the diff algorithm.
        // The key assertion is that "c" appears as added in the new side.
        let has_c = new.iter().any(|t| {
            (t.kind == ChangeKind::Addition || t.kind == ChangeKind::Rename) && t.text.contains('c')
        });
        assert!(has_c, "Expected 'c' to appear as an addition: {:?}", new);

        // If there are deletions, they should be paired as renames
        for t in &old_non_equal {
            assert_eq!(
                t.kind,
                ChangeKind::Rename,
                "Unpaired deletion found: {:?}",
                t
            );
        }
    }

    #[test]
    fn test_deletion_detection() {
        let (old, _new) = compute_token_changes("foo(a, b, c)", "foo(a, b)");
        // There should be deletions in old_tokens for the removed ", c" part
        let deletions: Vec<_> = old
            .iter()
            .filter(|t| t.kind == ChangeKind::Deletion || t.kind == ChangeKind::Rename)
            .collect();

        assert!(!deletions.is_empty(), "Expected deletion tokens in old");

        // The key assertion is that "c" appears as deleted in the old side
        let has_c = old.iter().any(|t| {
            (t.kind == ChangeKind::Deletion || t.kind == ChangeKind::Rename) && t.text.contains('c')
        });
        assert!(has_c, "Expected 'c' to appear as a deletion: {:?}", old);
    }

    #[test]
    fn test_complex_change() {
        let (old, new) = compute_token_changes(
            "let result = process data;",
            "let output = transform data opts;",
        );

        // "result" -> "output" and "process" -> "transform" should be renames
        let old_renames: Vec<_> = old
            .iter()
            .filter(|t| t.kind == ChangeKind::Rename)
            .collect();
        let new_renames: Vec<_> = new
            .iter()
            .filter(|t| t.kind == ChangeKind::Rename)
            .collect();

        assert!(
            old_renames.len() >= 2,
            "Expected at least 2 renames in old tokens, got {:?}",
            old_renames
        );
        assert!(
            new_renames.len() >= 2,
            "Expected at least 2 renames in new tokens, got {:?}",
            new_renames
        );

        // Check that the specific words are present as renames
        let old_rename_texts: Vec<&str> = old_renames.iter().map(|t| t.text.as_str()).collect();
        let new_rename_texts: Vec<&str> = new_renames.iter().map(|t| t.text.as_str()).collect();

        assert!(
            old_rename_texts.contains(&"result"),
            "Expected 'result' in old renames: {:?}",
            old_rename_texts
        );
        assert!(
            old_rename_texts.contains(&"process"),
            "Expected 'process' in old renames: {:?}",
            old_rename_texts
        );
        assert!(
            new_rename_texts.contains(&"output"),
            "Expected 'output' in new renames: {:?}",
            new_rename_texts
        );
        assert!(
            new_rename_texts.contains(&"transform"),
            "Expected 'transform' in new renames: {:?}",
            new_rename_texts
        );
    }
}
