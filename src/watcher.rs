use anyhow::Result;
use crossbeam_channel::Sender;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub enum WatchEvent {
    FilesChanged,
}

/// Start watching a directory for changes.
/// Returns the debouncer (must be kept alive).
pub fn start_watching(
    path: &Path,
    sender: Sender<WatchEvent>,
) -> Result<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>> {
    let mut debouncer = new_debouncer(
        Duration::from_millis(50),
        move |events: DebounceEventResult| {
            if let Ok(events) = events {
                let has_relevant = events.iter().any(|e| {
                    // Ignore .git directory changes
                    let is_git = e.path.components().any(|c| c.as_os_str() == ".git");
                    !is_git
                });
                if has_relevant {
                    let _ = sender.send(WatchEvent::FilesChanged);
                }
            }
        },
    )?;

    debouncer.watcher().watch(path, RecursiveMode::Recursive)?;
    Ok(debouncer)
}
