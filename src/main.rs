use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossbeam_channel::unbounded;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

mod app;
mod diff;
mod syntax;
mod ui;
mod watcher;

use app::App;
use diff::git2_provider::Git2Provider;
use diff::model::DiffMode;
use diff::provider::DiffProvider;

#[derive(Parser)]
#[command(name = "better-diff", about = "A better git diff viewer")]
struct Cli {
    #[arg(default_value = ".")]
    path: PathBuf,

    #[arg(short, long)]
    staged: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let repo_path = cli.path.canonicalize().unwrap_or(cli.path);
    let mut app = App::new(repo_path.clone());

    if cli.staged {
        app.mode = DiffMode::Staged;
    }

    // Compute initial diff
    let provider = Git2Provider::new();
    app.files = provider.compute_diff(&repo_path, app.mode)?;

    // Start file watcher
    let (watch_tx, watch_rx) = unbounded();
    let _watcher = watcher::start_watching(&repo_path, watch_tx)?;

    // Initialize terminal
    let mut terminal = ratatui::init();

    // Event loop
    let result = run_event_loop(&mut terminal, &mut app, &provider, &repo_path, &watch_rx);

    // Restore terminal
    ratatui::restore();

    result
}

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    provider: &Git2Provider,
    repo_path: &PathBuf,
    watch_rx: &crossbeam_channel::Receiver<watcher::WatchEvent>,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        // Drain file-system watch events and refresh diff
        while let Ok(_event) = watch_rx.try_recv() {
            let current_path = app.active_file().map(|f| f.path.clone());
            app.files = provider.compute_diff(repo_path, app.mode)?;
            // Try to keep the same file selected
            if let Some(path) = current_path {
                let new_index = app
                    .files
                    .iter()
                    .position(|f| f.path == path)
                    .unwrap_or(0);
                app.active_file = new_index;
            } else {
                app.active_file = 0;
            }
        }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

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
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        app.scroll_up();
                    }
                    KeyCode::Char('n') => {
                        app.next_hunk();
                    }
                    KeyCode::Char('N') => {
                        app.prev_hunk();
                    }
                    KeyCode::Char('s') => {
                        if app.mode != DiffMode::Staged {
                            app.mode = DiffMode::Staged;
                            app.files = provider.compute_diff(repo_path, app.mode)?;
                            app.active_file = 0;
                            app.scroll_offset = 0;
                        }
                    }
                    KeyCode::Char('w') => {
                        if app.mode != DiffMode::WorkingTree {
                            app.mode = DiffMode::WorkingTree;
                            app.files = provider.compute_diff(repo_path, app.mode)?;
                            app.active_file = 0;
                            app.scroll_offset = 0;
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
        }
    }
}
