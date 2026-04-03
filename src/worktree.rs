use std::path::{Path, PathBuf};

use anyhow::Result;
use git2::Repository;

pub enum WorktreeChange {
    Added(PathBuf),
    Removed(PathBuf),
}

pub struct WorktreeManager {
    main_repo_path: PathBuf,
    common_dir: PathBuf,
    worktrees: Vec<PathBuf>,
}

impl WorktreeManager {
    pub fn discover(repo_path: &Path) -> Result<Self> {
        let repo = Repository::discover(repo_path)?;

        let main_workdir = repo
            .workdir()
            .ok_or_else(|| anyhow::anyhow!("bare repositories are not supported"))?
            .to_path_buf();

        let common_dir = repo.commondir().to_path_buf();
        let worktrees = collect_all_worktree_paths(&repo, &main_workdir);

        Ok(Self {
            main_repo_path: main_workdir,
            common_dir,
            worktrees,
        })
    }

    pub fn refresh(&mut self) -> Vec<WorktreeChange> {
        let repo = match Repository::discover(&self.main_repo_path) {
            Ok(repo) => repo,
            Err(_) => return Vec::new(),
        };

        let new_worktrees = collect_all_worktree_paths(&repo, &self.main_repo_path);

        let mut changes = Vec::new();
        for path in &new_worktrees {
            if !self.worktrees.contains(path) {
                changes.push(WorktreeChange::Added(path.clone()));
            }
        }
        for path in &self.worktrees {
            if !new_worktrees.contains(path) {
                changes.push(WorktreeChange::Removed(path.clone()));
            }
        }

        self.worktrees = new_worktrees;
        changes
    }

    pub fn worktrees(&self) -> &[PathBuf] {
        &self.worktrees
    }

    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }
}

/// Enumerate all worktree paths: main worktree first, then linked sorted alphabetically.
fn collect_all_worktree_paths(repo: &Repository, main_workdir: &Path) -> Vec<PathBuf> {
    let mut linked = Vec::new();
    if let Ok(wt_names) = repo.worktrees() {
        for name in wt_names.iter().flatten() {
            if let Ok(wt) = repo.find_worktree(name) {
                linked.push(wt.path().to_path_buf());
            }
        }
        linked.sort();
    }

    let mut worktrees = vec![main_workdir.to_path_buf()];
    worktrees.extend(linked);
    worktrees
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature};

    fn setup_repo(path: &Path) -> Repository {
        let repo = Repository::init(path).expect("init repo");
        let mut config = repo.config().expect("config");
        config.set_str("user.name", "test").expect("name");
        config
            .set_str("user.email", "test@test.com")
            .expect("email");

        std::fs::write(path.join("file.txt"), "content\n").expect("write");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("file.txt")).expect("add");
        index.write().expect("write index");

        let tree_id = index.write_tree().expect("write tree");
        let sig = Signature::now("test", "test@test.com").expect("sig");
        {
            let tree = repo.find_tree(tree_id).expect("tree");
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .expect("commit");
        }

        repo
    }

    #[test]
    fn test_discover_single_worktree() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let _repo = setup_repo(tmp.path());

        let manager = WorktreeManager::discover(tmp.path()).expect("discover");
        assert_eq!(manager.worktrees().len(), 1);
        assert!(manager.worktrees()[0].exists());
    }

    #[test]
    fn test_discover_with_linked_worktrees() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let repo = setup_repo(tmp.path());

        let wt_path = tmp.path().join("wt1");
        repo.worktree("wt1", &wt_path, None)
            .expect("create worktree");

        let manager = WorktreeManager::discover(tmp.path()).expect("discover");
        assert_eq!(manager.worktrees().len(), 2);
        assert!(manager.worktrees()[0].exists());
        assert!(manager.worktrees()[1].exists());
    }

    #[test]
    fn test_discover_sorts_linked_alphabetically() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let repo = setup_repo(tmp.path());

        let wt_b = tmp.path().join("wt-b");
        let wt_a = tmp.path().join("wt-a");
        repo.worktree("wt-b", &wt_b, None)
            .expect("create worktree b");
        repo.worktree("wt-a", &wt_a, None)
            .expect("create worktree a");

        let manager = WorktreeManager::discover(tmp.path()).expect("discover");
        assert_eq!(manager.worktrees().len(), 3);
        // Linked worktrees should be sorted: wt-a before wt-b
        let wt1_name = manager.worktrees()[1]
            .file_name()
            .unwrap()
            .to_string_lossy();
        let wt2_name = manager.worktrees()[2]
            .file_name()
            .unwrap()
            .to_string_lossy();
        assert_eq!(wt1_name, "wt-a");
        assert_eq!(wt2_name, "wt-b");
    }

    #[test]
    fn test_discover_bare_repo_fails() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        Repository::init_bare(tmp.path()).expect("init bare");

        assert!(WorktreeManager::discover(tmp.path()).is_err());
    }

    #[test]
    fn test_refresh_detects_added_worktree() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let repo = setup_repo(tmp.path());

        let mut manager = WorktreeManager::discover(tmp.path()).expect("discover");
        assert_eq!(manager.worktrees().len(), 1);

        let wt_path = tmp.path().join("wt1");
        repo.worktree("wt1", &wt_path, None)
            .expect("create worktree");

        let changes = manager.refresh();
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], WorktreeChange::Added(_)));
        assert_eq!(manager.worktrees().len(), 2);
    }

    #[test]
    fn test_refresh_detects_removed_worktree() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let repo = setup_repo(tmp.path());

        let wt_path = tmp.path().join("wt1");
        repo.worktree("wt1", &wt_path, None)
            .expect("create worktree");

        let mut manager = WorktreeManager::discover(tmp.path()).expect("discover");
        assert_eq!(manager.worktrees().len(), 2);

        let wt = repo.find_worktree("wt1").expect("find wt");
        let mut opts = git2::WorktreePruneOptions::new();
        opts.working_tree(true);
        opts.valid(true);
        wt.prune(Some(&mut opts)).expect("prune");

        let changes = manager.refresh();
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], WorktreeChange::Removed(_)));
        assert_eq!(manager.worktrees().len(), 1);
    }

    #[test]
    fn test_refresh_no_changes() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let _repo = setup_repo(tmp.path());

        let mut manager = WorktreeManager::discover(tmp.path()).expect("discover");
        let changes = manager.refresh();
        assert!(changes.is_empty());
    }

    #[test]
    fn test_common_dir_is_shared_git_dir() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let _repo = setup_repo(tmp.path());

        let manager = WorktreeManager::discover(tmp.path()).expect("discover");
        let common_dir = manager.common_dir();
        assert!(common_dir.exists());
        assert!(common_dir.join("HEAD").exists());
    }
}
