use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use git2::Repository;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};

use better_diff::app::{App, WorktreeContext};
use better_diff::diff::git2_provider::Git2Provider;
use better_diff::diff::model::DiffMode;
use better_diff::ui;
use better_diff::ui::animation::AnimationState;
use better_diff::watcher::{WatchEvent, WatcherSet};
use better_diff::worktree::{WorktreeChange, WorktreeManager};

#[derive(Parser)]
#[command(name = "better-diff", about = "A better git diff viewer")]
struct Cli {
    #[arg(default_value = ".")]
    path: PathBuf,

    #[arg(short, long)]
    staged: bool,

    #[arg(long, help = "Start in live mode (auto-follow file changes)")]
    live: bool,

    #[arg(long, value_enum, help = "Generate shell completions")]
    completions: Option<clap_complete::Shell>,

    #[arg(long, help = "Compare two refs (e.g., main..feature, HEAD~3..HEAD)")]
    compare: Option<String>,
}

struct TerminalGuard {
    terminal: ratatui::DefaultTerminal,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        ratatui::crossterm::execute!(
            std::io::stdout(),
            ratatui::crossterm::event::EnableMouseCapture
        )?;

        let terminal = ratatui::init();
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = ratatui::crossterm::execute!(
            std::io::stdout(),
            ratatui::crossterm::event::DisableMouseCapture
        );
        ratatui::restore();
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(shell) = cli.completions {
        clap_complete::generate(
            shell,
            &mut Cli::command(),
            "better-diff",
            &mut std::io::stdout(),
        );
        return Ok(());
    }

    let repo_path = cli.path.canonicalize().unwrap_or(cli.path);

    // Discover all worktrees
    let manager = WorktreeManager::discover(&repo_path)?;

    // Create contexts for each worktree and compute initial diffs
    let provider = Git2Provider::new();
    let mut contexts: Vec<WorktreeContext> = Vec::new();
    for wt_path in manager.worktrees() {
        let repo = Repository::discover(wt_path)?;
        let mut ctx = WorktreeContext::new(wt_path.clone(), &repo);
        drop(repo);
        ctx.recompute(&provider)?;
        contexts.push(ctx);
    }

    // Match CLI path to set active worktree (repo_path is already canonicalized)
    let active_worktree = manager
        .worktrees()
        .iter()
        .position(|p| p.canonicalize().unwrap_or_else(|_| p.clone()) == repo_path)
        .unwrap_or(0);

    // Apply CLI mode to the initially active worktree
    if let Some(compare) = cli.compare {
        let (from_ref, to_ref) = parse_ref_range(&compare);
        contexts[active_worktree].mode = DiffMode::Commits { from_ref, to_ref };
        contexts[active_worktree].recompute(&provider)?;
    } else if cli.staged {
        contexts[active_worktree].mode = DiffMode::Staged;
        contexts[active_worktree].recompute(&provider)?;
    }

    // Start watchers for all worktrees + .git/worktrees/
    let (mut watcher_set, watch_rx) =
        WatcherSet::new(manager.worktrees(), manager.common_dir())?;

    let mut app = App {
        contexts,
        active_worktree,
        should_quit: false,
        manager,
        live_mode: cli.live,
    };

    let mut terminal = TerminalGuard::new()?;
    run_event_loop(
        &mut terminal.terminal,
        &mut app,
        &provider,
        &watch_rx,
        &mut watcher_set,
    )
}

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    provider: &Git2Provider,
    watch_rx: &crossbeam_channel::Receiver<WatchEvent>,
    watcher_set: &mut WatcherSet,
) -> Result<()> {
    loop {
        let visible_rows = content_visible_rows(terminal.size()?.height);

        clamp_active(app.active_context_mut(), visible_rows);

        // Drain all pending watch events
        let mut recompute_indices: Vec<usize> = Vec::new();
        let mut live_target: Option<(usize, Vec<PathBuf>)> = None;
        let mut worktree_list_changed = false;
        let current_gen = watcher_set.generation();

        while let Ok(event) = watch_rx.try_recv() {
            match event {
                WatchEvent::FilesChanged {
                    worktree_index,
                    generation,
                    changed_paths,
                } => {
                    if generation == current_gen && worktree_index < app.contexts.len() {
                        if !recompute_indices.contains(&worktree_index) {
                            recompute_indices.push(worktree_index);
                        }
                        if app.live_mode {
                            live_target = Some((worktree_index, changed_paths));
                        }
                    }
                }
                WatchEvent::WorktreeListChanged => {
                    worktree_list_changed = true;
                }
            }
        }

        if worktree_list_changed {
            handle_worktree_changes(app, provider, watcher_set)?;
            recompute_indices.clear();
            live_target = None;
        }

        for idx in recompute_indices {
            if idx < app.contexts.len() {
                let _ = app.contexts[idx].recompute(provider);
            }
        }

        // Live mode: navigate to the most recently changed file
        if let Some((wt_idx, paths)) = live_target {
            if wt_idx < app.contexts.len() {
                app.active_worktree = wt_idx;
                let ctx = &mut app.contexts[wt_idx];

                if let Some(file_idx) = ctx.find_file_by_paths(&paths) {
                    ctx.active_file = file_idx;
                    ctx.scroll_offset = 0;

                    if ui::ensure_active_file_layout(ctx) {
                        if let Some(&first_hunk) =
                            ctx.render_cache.layout.hunk_starts().first()
                        {
                            ctx.scroll_offset = first_hunk;
                        }
                    }
                }
            }
        }

        clamp_active(app.active_context_mut(), visible_rows);

        if event::poll(Duration::from_millis(50))? {
            let mut needs_clamp = false;
            match event::read()? {
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        let ctx = app.active_context_mut();
                        for _ in 0..3 {
                            ctx.scroll_down();
                        }
                        needs_clamp = true;
                    }
                    MouseEventKind::ScrollUp => {
                        let ctx = app.active_context_mut();
                        for _ in 0..3 {
                            ctx.scroll_up();
                        }
                        needs_clamp = true;
                    }
                    _ => {}
                },
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.should_quit = true;
                        }
                        KeyCode::Char(']') => {
                            app.next_worktree();
                            needs_clamp = true;
                        }
                        KeyCode::Char('G') => {
                            let ctx = app.active_context_mut();
                            if ui::ensure_active_file_layout(ctx) {
                                let total_lines = ctx.render_cache.layout.total_lines();
                                ctx.scroll_to_bottom(total_lines, visible_rows);
                                needs_clamp = true;
                            }
                        }
                        KeyCode::Char('g') => {
                            app.active_context_mut().scroll_to_top();
                            needs_clamp = true;
                        }
                        KeyCode::Tab => {
                            app.active_context_mut().next_file();
                            needs_clamp = true;
                        }
                        KeyCode::BackTab => {
                            app.active_context_mut().prev_file();
                            needs_clamp = true;
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            app.active_context_mut().scroll_down();
                            needs_clamp = true;
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            app.active_context_mut().scroll_up();
                            needs_clamp = true;
                        }
                        KeyCode::PageDown => {
                            let ctx = app.active_context_mut();
                            if ctx.active_file().is_some() {
                                let step = visible_rows.max(1).div_euclid(2).max(1);
                                ctx.scroll_page_down(step);
                                needs_clamp = true;
                            }
                        }
                        KeyCode::PageUp => {
                            let ctx = app.active_context_mut();
                            if ctx.active_file().is_some() {
                                let step = visible_rows.max(1).div_euclid(2).max(1);
                                ctx.scroll_page_up(step);
                                needs_clamp = true;
                            }
                        }
                        KeyCode::Home => {
                            app.active_context_mut().scroll_to_top();
                            needs_clamp = true;
                        }
                        KeyCode::End => {
                            let ctx = app.active_context_mut();
                            if ui::ensure_active_file_layout(ctx) {
                                let total_lines = ctx.render_cache.layout.total_lines();
                                ctx.scroll_to_bottom(total_lines, visible_rows);
                                needs_clamp = true;
                            }
                        }
                        KeyCode::Char('n') => {
                            let ctx = app.active_context_mut();
                            if ui::ensure_active_file_layout(ctx) {
                                let total_lines = ctx.render_cache.layout.total_lines();
                                let hunk_starts = ctx.render_cache.layout.hunk_starts().to_vec();
                                ctx.next_hunk_with_offsets(
                                    &hunk_starts,
                                    total_lines,
                                    visible_rows,
                                );
                                ctx.animation = Some(AnimationState::new());
                                needs_clamp = true;
                            }
                        }
                        KeyCode::Char('N') => {
                            let ctx = app.active_context_mut();
                            if ui::ensure_active_file_layout(ctx) {
                                let total_lines = ctx.render_cache.layout.total_lines();
                                let hunk_starts = ctx.render_cache.layout.hunk_starts().to_vec();
                                ctx.prev_hunk_with_offsets(
                                    &hunk_starts,
                                    total_lines,
                                    visible_rows,
                                );
                                ctx.animation = Some(AnimationState::new());
                                needs_clamp = true;
                            }
                        }
                        KeyCode::Char('s') => {
                            let ctx = app.active_context_mut();
                            if ctx.set_mode(DiffMode::Staged) {
                                ctx.recompute(provider)?;
                                needs_clamp = true;
                            }
                        }
                        KeyCode::Char('w') => {
                            let ctx = app.active_context_mut();
                            if ctx.set_mode(DiffMode::WorkingTree) {
                                ctx.recompute(provider)?;
                                needs_clamp = true;
                            }
                        }
                        KeyCode::Char('c') => {
                            app.active_context_mut().cycle_collapse();
                            needs_clamp = true;
                        }
                        KeyCode::Char('L') => {
                            app.live_mode = !app.live_mode;
                        }
                        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                            let index = (c as usize) - ('1' as usize);
                            app.active_context_mut().select_file(index);
                            needs_clamp = true;
                        }
                        _ => {}
                    }

                    if app.should_quit {
                        return Ok(());
                    }
                }
                _ => {}
            }

            if needs_clamp {
                clamp_active(app.active_context_mut(), visible_rows);
            }
        }

        terminal.draw(|frame| ui::render(frame, app))?;

        // Clean up completed animations
        let ctx = app.active_context_mut();
        if let Some(ref anim) = ctx.animation
            && anim.is_done()
        {
            ctx.animation = None;
        }
    }
}

fn handle_worktree_changes(
    app: &mut App,
    provider: &Git2Provider,
    watcher_set: &mut WatcherSet,
) -> Result<()> {
    let changes = app.manager.refresh();
    for change in changes {
        match change {
            WorktreeChange::Added(path) => {
                if let Ok(repo) = Repository::discover(&path) {
                    let mut ctx = WorktreeContext::new(path.clone(), &repo);
                    drop(repo);
                    if ctx.recompute(provider).is_ok() && watcher_set.add_worktree(&path).is_ok() {
                        app.contexts.push(ctx);
                    }
                }
            }
            WorktreeChange::Removed(path) => {
                if let Some(idx) = app.remove_context_by_path(&path) {
                    watcher_set.remove_worktree(idx);
                }
            }
        }
    }
    Ok(())
}

fn clamp_active(ctx: &mut WorktreeContext, visible_rows: usize) {
    if ui::ensure_active_file_layout(ctx) {
        let total_lines = ctx.render_cache.layout.total_lines();
        ctx.clamp_scroll_offset(total_lines, visible_rows);
    } else {
        ctx.scroll_offset = 0;
    }
}

fn content_visible_rows(terminal_height: u16) -> usize {
    terminal_height.saturating_sub(5) as usize
}

/// Parse a ref range like "main..feature" or a single ref like "HEAD~3".
/// A single ref is treated as "ref^..ref" (compare with its parent).
fn parse_ref_range(spec: &str) -> (String, String) {
    if let Some((from, to)) = spec.split_once("..") {
        (from.to_string(), to.to_string())
    } else {
        // Single ref: compare with its parent
        (format!("{}^", spec), spec.to_string())
    }
}
