# better-diff

A Rust TUI program for visualizing git diffs with novel visualization features.

![better-diff screenshot](assets/screenshot.png)

## Features

- **Side-by-side diff view** with file tabs
- **Token-level change highlighting** — word-level diffs within changed lines (not just line-level)
- **Structural folding** — AST-aware collapsing via tree-sitter
- **Block move detection** — detects moved code blocks with visual linking
- **Heat map minimap** — change density sidebar
- **Change animations** — subtle transitions on hunk focus
- **Syntax highlighting** — tree-sitter powered for 18 languages
- **Live mode** — auto-follows file changes across worktrees, scrolls to the first hunk (great for watching agents work)
- **Search** — case-insensitive live search with match highlighting and navigation
- **Theming** — dark, light, and monokai themes (config or `--theme` flag)
- **Responsive layout** — side-by-side when wide, unified view when narrow (<120 cols)
- **Stdin/pipe support** — read unified diffs from stdin (`git diff | better-diff`)
- **Commit comparison** — compare branches, commits, or ranges (`--compare main..feature`)
- **Two modes** — working tree vs HEAD, staged vs HEAD
- **Filesystem watching** — near-real-time updates via `notify`
- **Git worktree support** — switch between worktrees with `]`
- **Shell completions** — `--completions` for bash, zsh, fish, etc.
- **Config file** — persistent defaults via `~/.config/better-diff/config.toml`

## Install

```sh
cargo install --path .
```

## Usage

```sh
# View working tree changes
better-diff

# View staged changes
better-diff --staged

# Compare branches or commits
better-diff --compare main..feature
better-diff --compare HEAD~3..HEAD

# View a single commit (compares with its parent)
better-diff --compare HEAD~1

# View changes in a specific directory
better-diff /path/to/repo

# Start in live mode (auto-follow file changes)
better-diff --live

# Use a different theme
better-diff --theme monokai

# Read diff from stdin
git diff main..feature | better-diff
cat changes.patch | better-diff

# Generate shell completions
better-diff --completions zsh > ~/.zsh/completions/_better-diff
```

## Keybindings

| Key | Action |
|---|---|
| `j`/`k` or arrows | Scroll line by line |
| `PgUp`/`PgDn` | Scroll by half page |
| `n`/`N` | Next/previous hunk (or search match) |
| `g`/`G` | Jump to top/bottom |
| `Home`/`End` | Jump to top/bottom |
| `/` | Search (case-insensitive, live results) |
| `Tab`/`Shift+Tab` | Next/previous file |
| `1-9` | Jump to file by number |
| `s` / `w` | Staged / working tree mode |
| `c` | Cycle collapse level (tight/scoped/expanded) |
| `L` | Toggle live mode |
| `]` | Switch worktree (when multiple are open) |
| `q` / `Esc` | Quit |

## Syntax Highlighting

Syntax highlighting is automatic based on file extension:

| Language | Extensions |
|---|---|
| Rust | `.rs` |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` |
| TypeScript | `.ts`, `.tsx`, `.mts`, `.cts` |
| Python | `.py`, `.pyi` |
| Go | `.go` |
| C | `.c`, `.h` |
| C++ | `.cc`, `.cpp`, `.cxx`, `.hpp`, `.hxx`, `.hh` |
| Java | `.java` |
| C# | `.cs` |
| Ruby | `.rb`, `.rake`, `.gemspec` |
| Bash/Shell | `.sh`, `.bash`, `.zsh` |
| Lua | `.lua` |
| Zig | `.zig` |
| Elixir | `.ex`, `.exs` |
| JSON | `.json`, `.jsonc` |
| TOML | `.toml` |
| HTML | `.html`, `.htm` |
| CSS | `.css`, `.scss` |

Also highlights `Dockerfile`, `Containerfile`, `Makefile`, and `GNUmakefile` by name.

## Configuration

Create `~/.config/better-diff/config.toml` to set defaults:

```toml
# Start in live mode by default
live = false

# Default to staged view
staged = false

# Default collapse level: "tight", "scoped", or "expanded"
collapse = "scoped"

# Color theme: "dark", "light", or "monokai"
theme = "dark"
```

CLI flags always override config file values.

## Built With

- [Ratatui](https://ratatui.rs/) — TUI framework
- [git2](https://docs.rs/git2) — Git operations
- [similar](https://docs.rs/similar) — Word/char-level diffing
- [tree-sitter](https://tree-sitter.github.io/) — AST parsing & syntax highlighting
- [notify](https://docs.rs/notify) — Filesystem watching
