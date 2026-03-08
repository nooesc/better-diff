use std::process::Command;
use tempfile::TempDir;

fn setup_repo_with_changes() -> TempDir {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    Command::new("git")
        .args(["init"])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(path)
        .output()
        .unwrap();

    // Create initial Rust file
    std::fs::write(
        path.join("main.rs"),
        r#"fn main() {
    let x = foo(a, b);
    println!("{}", x);
}

fn helper() {
    do_thing_a();
    do_thing_b();
    do_thing_c();
    do_thing_d();
}
"#,
    )
    .unwrap();

    Command::new("git")
        .args(["add", "."])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(path)
        .output()
        .unwrap();

    // Make changes
    std::fs::write(
        path.join("main.rs"),
        r#"fn main() {
    let x = bar(a, b, c);
    println!("{}", x);
}

fn setup() {
    init_config();
    init_logging();
    init_database();
    init_server();
}

fn helper() {
    do_thing_a();
    do_thing_b();
    do_thing_c();
    do_thing_d();
}
"#,
    )
    .unwrap();

    dir
}

#[test]
fn test_full_diff_pipeline() {
    use better_diff::diff::git2_provider::Git2Provider;
    use better_diff::diff::model::*;
    use better_diff::diff::provider::DiffProvider;

    let dir = setup_repo_with_changes();
    let provider = Git2Provider::new();
    let files = provider
        .compute_diff(dir.path(), DiffMode::WorkingTree)
        .unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, FileStatus::Modified);
    assert!(!files[0].hunks.is_empty());

    // Check that token-level diffs were computed
    let has_tokens = files[0]
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .any(|l| !l.tokens.is_empty());
    assert!(
        has_tokens,
        "Should have token-level diffs for modified lines"
    );

    // Check that fold regions were computed
    assert!(
        !files[0].fold_regions.is_empty(),
        "Should detect foldable regions in Rust code"
    );
}
