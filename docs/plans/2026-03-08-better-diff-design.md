# better-diff Design Document

A Rust TUI program for visualizing git diffs with novel visualization features.

## Goals

- Real-time side-by-side diff viewer with file tabs
- Token-level change highlighting (not just line-level)
- AST-aware structural folding via tree-sitter
- Block move detection with visual linking
- Heat map minimap for change density
- Subtle change animations on hunk focus
- Two modes: working tree vs HEAD, staged vs unstaged
- Filesystem watching for near-real-time updates

## Architecture

Three decoupled layers:

1. **Input layer** — `notify` watches filesystem, CLI args set mode. Events flow via `crossbeam-channel`.
2. **Diff engine** — `git2` for line-level diffs, `similar` for word-level diffs within changed lines, `tree-sitter` for AST context. Abstracted behind a `DiffProvider` trait for future extensibility (e.g., difftastic backend).
3. **TUI layer** — Ratatui renders from a `DiffState` struct. Immediate-mode rendering on every state change.

Event loop: single-threaded using `crossbeam-channel` `select!` over filesystem events, keyboard input, and a tick timer for animations.

## TUI Layout

```
┌─[better-diff]──────────────────────────────────────────────┐
│ file1.rs │ file2.rs │ file3.rs                             │
│ [Working Tree ▾]                              [3 files ±]  │
├────────────────────────────┬───────────────────────────────┤
│ old content                │ new content                ▐█▌│
│                            │                            ▐ ▌│
│  unchanged lines...        │  unchanged lines...        ▐ ▌│
│┈┈┈┈ fn name (N lines) ┈┈┈┈│┈┈┈┈ fn name (N lines) ┈┈┈┈▐ ▌│
│- old line                  │+ new line                  ▐█▌│
│                            │                            ▐ ▌│
├────────────────────────────┴───────────────────────────────┤
│ [h]elp  [q]uit  [tab] next file  [s]taged  [w]orking tree │
└────────────────────────────────────────────────────────────┘
```

## Navigation

| Key | Action |
|---|---|
| `j`/`k` or arrows | Scroll line by line |
| `n`/`N` | Jump to next/previous hunk |
| `Tab`/`Shift+Tab` | Next/previous file |
| `1-9` | Jump to file by number |
| `s` / `w` | Staged / working tree mode |
| `Enter` | Expand collapsed region |
| `c` | Cycle collapse level |
| `q` / `Esc` | Quit |

## Visualization Features (in priority order)

### 1. Token-level highlighting (core)

Word-level diff within changed lines using `similar`. Color-coded by change type:
- Rename (blue) — token replaced
- Addition (green) — tokens inserted
- Deletion (red) — tokens removed
- Move (purple) — detected via hash matching

Subtle dim background on the line, bright foreground on specific changed tokens.

### 2. Structural folding (core)

Tree-sitter AST determines fold boundaries. Collapse labels show scope context:
```
┈┈┈┈ fn setup_database() (lines 14-28) ┈┈┈┈
┈┈┈┈ impl Server { ... } (lines 30-180) ┈┈┈┈
```

Three levels: tight (3-line context), scoped (AST nodes), expanded (everything).

### 3. Block move detection (enhancement)

Hash-match normalized blocks of 3+ lines. Annotate with source/destination:
```
-  ┌─── moved to line 145 ───┐
+  ┌─── moved from line 32 ──┐
```

Works within and across files.

### 4. Heat map minimap (enhancement)

Right-edge sidebar proportional to file length. Colored blocks show change density. Doubles as scrollbar.

### 5. Change animation (polish)

150ms transition on hunk focus: deleted tokens fade out, added tokens fade in, renames crossfade. Implemented via style cycling on tick timer.

## Data Model

```rust
struct App {
    mode: DiffMode,              // WorkingTree | Staged
    files: Vec<FileDiff>,
    active_file: usize,
    collapse_level: CollapseLevel,
    watcher: FileWatcher,
    animation_state: Option<AnimationState>,
}

struct FileDiff {
    path: PathBuf,
    status: FileStatus,          // Modified | Added | Deleted | Renamed
    hunks: Vec<Hunk>,
    old_content: String,
    new_content: String,
    ast_old: Option<Tree>,
    ast_new: Option<Tree>,
    fold_regions: Vec<FoldRegion>,
    move_matches: Vec<MoveMatch>,
}

struct Hunk {
    old_start: usize,
    new_start: usize,
    lines: Vec<DiffLine>,
}

struct DiffLine {
    kind: LineKind,              // Context | Added | Deleted | Modified
    old_text: Option<String>,
    new_text: Option<String>,
    tokens: Vec<TokenChange>,
}

struct TokenChange {
    kind: ChangeKind,            // Rename | Addition | Deletion | Move
    old_span: Option<Range<usize>>,
    new_span: Option<Range<usize>>,
}
```

### Diff pipeline

1. `git2` detects changed files, produces line-level hunks
2. `similar` computes word-level diffs within modified line pairs
3. `tree-sitter` parses old + new content for fold regions and syntax highlighting
4. Move detector hashes normalized blocks, matches above similarity threshold
5. Results packed into `FileDiff`, stored in `App` state
6. On filesystem event, re-run for affected files only (incremental)

### DiffProvider trait

```rust
trait DiffProvider {
    fn compute_diff(&self, repo: &Repository, mode: DiffMode) -> Result<Vec<FileDiff>>;
}
```

Implementations: `Git2Provider` (default), `DifftasticProvider` (future).

## Dependencies

| Crate | Purpose |
|---|---|
| `ratatui` 0.30 | TUI framework |
| `crossterm` | Terminal backend |
| `git2` 0.20 | Git operations |
| `similar` | Word/char-level diffing |
| `tree-sitter` | AST parsing |
| `tree-sitter-rust` | Rust grammar (add more later) |
| `notify` 8.x | Filesystem watching |
| `crossbeam-channel` | Event loop channels |
| `clap` 4.x | CLI argument parsing |
| `anyhow` | Error handling |

## Project Structure

```
better-diff/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point, CLI args, event loop
│   ├── app.rs               # App state, mode switching, navigation
│   ├── diff/
│   │   ├── mod.rs            # DiffProvider trait
│   │   ├── git2_provider.rs  # git2 + similar based diffing
│   │   ├── model.rs          # FileDiff, Hunk, DiffLine, TokenChange
│   │   ├── moves.rs          # Block move detection
│   │   └── folding.rs        # AST-based fold region computation
│   ├── ui/
│   │   ├── mod.rs            # Top-level render function
│   │   ├── tabs.rs           # File tab bar widget
│   │   ├── split_pane.rs     # Side-by-side diff view
│   │   ├── minimap.rs        # Heat map sidebar
│   │   ├── status_bar.rs     # Bottom keybinding hints
│   │   └── animation.rs      # Change transition effects
│   ├── watcher.rs            # notify integration, event debouncing
│   └── syntax.rs             # tree-sitter parsing & highlighting
```

## Key Decisions

- `diff/` and `ui/` fully decoupled
- Start with Rust grammar only, add languages via feature flags
- Debounce filesystem events at 50ms
- Build features in priority order: token highlighting → structural folding → move detection → minimap → animations
- Design for personal use first, architect cleanly enough to open-source later
