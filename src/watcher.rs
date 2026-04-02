use anyhow::Result;
use crossbeam_channel::Sender;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};
use std::path::{Path, PathBuf};
use std::time::Duration;

type WatchDebouncer = notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>;

#[derive(Debug)]
pub enum WatchEvent {
    FilesChanged {
        worktree_index: usize,
        generation: u64,
    },
    WorktreeListChanged,
}

/// Manages one watcher per worktree plus an optional watcher for `.git/worktrees/`.
pub struct WatcherSet {
    watchers: Vec<(PathBuf, WatchDebouncer)>,
    #[allow(dead_code)]
    git_worktrees_watcher: Option<WatchDebouncer>,
    sender: Sender<WatchEvent>,
    generation: u64,
}

impl WatcherSet {
    pub fn new(
        worktrees: &[PathBuf],
        common_dir: &Path,
    ) -> Result<(Self, crossbeam_channel::Receiver<WatchEvent>)> {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let generation = 0u64;

        let mut watchers = Vec::new();
        for (index, path) in worktrees.iter().enumerate() {
            let debouncer = create_worktree_watcher(index, path, &sender, generation)?;
            watchers.push((path.clone(), debouncer));
        }

        let worktrees_dir = common_dir.join("worktrees");
        let git_worktrees_watcher = if worktrees_dir.exists() {
            Some(create_worktrees_dir_watcher(&worktrees_dir, &sender)?)
        } else {
            None
        };

        Ok((
            Self {
                watchers,
                git_worktrees_watcher,
                sender,
                generation,
            },
            receiver,
        ))
    }

    pub fn add_worktree(&mut self, path: &Path) -> Result<()> {
        self.generation += 1;
        let index = self.watchers.len();
        let debouncer = create_worktree_watcher(index, path, &self.sender, self.generation)?;
        self.watchers.push((path.to_path_buf(), debouncer));
        Ok(())
    }

    pub fn remove_worktree(&mut self, index: usize) {
        self.generation += 1;
        self.watchers.remove(index);

        // Rebuild all remaining watchers with correct indices
        let paths: Vec<PathBuf> = self.watchers.drain(..).map(|(p, _)| p).collect();
        for (i, path) in paths.into_iter().enumerate() {
            if let Ok(debouncer) =
                create_worktree_watcher(i, &path, &self.sender, self.generation)
            {
                self.watchers.push((path, debouncer));
            }
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

fn create_worktree_watcher(
    index: usize,
    path: &Path,
    sender: &Sender<WatchEvent>,
    generation: u64,
) -> Result<WatchDebouncer> {
    let sender = sender.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(50),
        move |events: DebounceEventResult| {
            if let Ok(events) = events {
                let has_relevant = events.iter().any(|e| {
                    let is_git = e.path.components().any(|c| c.as_os_str() == ".git");
                    !is_git
                });
                if has_relevant {
                    let _ = sender.send(WatchEvent::FilesChanged {
                        worktree_index: index,
                        generation,
                    });
                }
            }
        },
    )?;
    debouncer.watcher().watch(path, RecursiveMode::Recursive)?;
    Ok(debouncer)
}

fn create_worktrees_dir_watcher(
    worktrees_dir: &Path,
    sender: &Sender<WatchEvent>,
) -> Result<WatchDebouncer> {
    let sender = sender.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(200),
        move |events: DebounceEventResult| {
            if events.is_ok() {
                let _ = sender.send(WatchEvent::WorktreeListChanged);
            }
        },
    )?;
    debouncer
        .watcher()
        .watch(worktrees_dir, RecursiveMode::Recursive)?;
    Ok(debouncer)
}
