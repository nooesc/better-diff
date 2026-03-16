use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossbeam_channel::unbounded;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};

use better_diff::app::App;
use better_diff::diff::git2_provider::Git2Provider;
use better_diff::diff::model::DiffMode;
use better_diff::diff::provider::DiffProvider;
use better_diff::ui;
use better_diff::watcher;

#[derive(Parser)]
#[command(name = "better-diff", about = "A better git diff viewer")]
struct Cli {
    #[arg(default_value = ".")]
    path: PathBuf,

    #[arg(short, long)]
    staged: bool,
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

    let repo_path = cli.path.canonicalize().unwrap_or(cli.path);
    let mut app = App::new(repo_path.clone());
    app.branch_label = app.resolve_branch_label();

    if cli.staged {
        app.mode = DiffMode::Staged;
    }

    // Compute initial diff
    let provider = Git2Provider::new();
    app.files = provider.compute_diff(&repo_path, app.mode)?;

    // Start file watcher
    let (watch_tx, watch_rx) = unbounded();
    let _watcher = watcher::start_watching(&repo_path, watch_tx)?;

    let mut terminal = TerminalGuard::new()?;
    // Event loop
    let result = run_event_loop(&mut terminal.terminal, &mut app, &provider, &repo_path, &watch_rx);
    result
}

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    provider: &Git2Provider,
    repo_path: &Path,
    watch_rx: &crossbeam_channel::Receiver<watcher::WatchEvent>,
) -> Result<()> {
    let mut recompute = |app: &mut App, provider: &Git2Provider, repo_path: &Path| -> Result<()> {
        let prev_path = app.active_file().map(|f| f.path.clone());
        let prev_old_path = app.active_file().and_then(|f| f.old_path.clone());

        app.files = provider.compute_diff(repo_path, app.mode)?;
        app.render_cache.invalidate();
        app.branch_label = app.resolve_branch_label();

        if app.files.is_empty() {
            app.active_file = 0;
            app.scroll_offset = 0;
            return Ok(());
        }

        let new_index = if let Some(path) = prev_path {
            app.files
                .iter()
                .position(|f| {
                    f.path == path || prev_old_path.as_ref().is_some_and(|p| f.old_path.as_ref() == Some(p))
                })
                .unwrap_or(0)
        } else {
            0
        };
        app.active_file = new_index;

        Ok(())
    };

    loop {
        let visible_rows = content_visible_rows(terminal.size()?.height);

        let mut clamp_active = |app: &mut App| {
            if let Some(file) = app.active_file() {
                let (total_lines, _) = ui::split_pane::rendered_file_layout(file, app.collapse_level);
                app.clamp_scroll_offset(total_lines, visible_rows);
            } else {
                app.scroll_offset = 0;
            }
        };
        clamp_active(app);

        terminal.draw(|frame| ui::render(frame, app))?;

        // Drain all pending watch events and refresh diff at most once
        let mut needs_recompute = false;
        while watch_rx.try_recv().is_ok() {
            needs_recompute = true;
        }
        if needs_recompute {
            recompute(app, provider, repo_path)?;
            clamp_active(app);
        }

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        for _ in 0..3 {
                            app.scroll_down();
                        }
                        clamp_active(app);
                    }
                    MouseEventKind::ScrollUp => {
                        for _ in 0..3 {
                            app.scroll_up();
                        }
                        clamp_active(app);
                    }
                    _ => {}
                },
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.should_quit = true;
                        }
                        KeyCode::Tab => {
                            app.next_file();
                        }
                        KeyCode::BackTab => {
                            app.prev_file();
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            app.scroll_down();
                            clamp_active(app);
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            app.scroll_up();
                            clamp_active(app);
                        }
                        KeyCode::Char('n') => {
                            if let Some(file) = app.active_file() {
                                let (total_lines, hunk_starts) =
                                    ui::split_pane::rendered_file_layout(file, app.collapse_level);

                                app.next_hunk_with_offsets(&hunk_starts, total_lines, visible_rows);
                                app.animation = Some(crate::ui::animation::AnimationState::new());
                                clamp_active(app);
                            }
                        }
                        KeyCode::Char('N') => {
                            if let Some(file) = app.active_file() {
                                let (total_lines, hunk_starts) =
                                    ui::split_pane::rendered_file_layout(file, app.collapse_level);

                                app.prev_hunk_with_offsets(&hunk_starts, total_lines, visible_rows);
                                app.animation = Some(crate::ui::animation::AnimationState::new());
                                clamp_active(app);
                            }
                        }
                        KeyCode::Char('s') => {
                            if app.set_mode(DiffMode::Staged) {
                                recompute(app, provider, repo_path)?;
                                clamp_active(app);
                            }
                        }
                        KeyCode::Char('w') => {
                            if app.set_mode(DiffMode::WorkingTree) {
                                recompute(app, provider, repo_path)?;
                                clamp_active(app);
                            }
                        }
                        KeyCode::Char('c') => {
                            app.cycle_collapse();
                        }
                        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                            let index = (c as usize) - ('1' as usize);
                            app.select_file(index);
                        }
                        _ => {}
                    }

                    if app.should_quit {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }

        // Clean up completed animations
        if let Some(ref anim) = app.animation
            && anim.is_done()
        {
            app.animation = None;
        }
    }
}

fn content_visible_rows(terminal_height: u16) -> usize {
    terminal_height.saturating_sub(5) as usize
}
