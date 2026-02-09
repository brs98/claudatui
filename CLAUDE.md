# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

claudatui is a Rust TUI for browsing, managing, and interacting with Claude Code conversations. It spawns Claude processes in PTYs, renders their output via a VT100 parser, and provides vim-like navigation with sidebar, bookmarks, search, split panes, and archive management.

## Build & Development Commands

```bash
cargo build                                    # Debug build
cargo build --release                          # Release build (thin LTO, stripped)
cargo run                                      # Run in debug mode
RUST_LOG=debug cargo run                       # Run with debug logging
cargo test --all-features                      # Run all tests
cargo fmt --all -- --check                     # Check formatting (CI uses this)
cargo clippy --all-targets --all-features -- -D warnings  # Lint (CI uses this)
```

CI runs formatting, clippy, tests (stable + beta), and a release build on every PR to main.

## Architecture

### Event Loop (`event_loop.rs`)

The core loop runs with a 50ms poll timeout. Each tick: checks timeouts (chord/leader/escape), processes PTY output, updates session state caches, polls JSONL status for running sessions, checks for sessions-index.json changes via file watcher, updates toasts, renders UI, then polls for crossterm events.

### State Container (`app/`)

A single `App` struct holds all application state. It's decomposed into focused modules:
- `actions.rs` — User-facing actions (archive, bookmark, clipboard, open session)
- `navigation.rs` — Sidebar navigation (up/down, jump, filtering)
- `panes.rs` — Split-pane management (dual terminal panes)
- `sessions.rs` — Session lifecycle (start, close, resume)
- `state.rs` — Type definitions (ClipboardStatus, TextSelection, SplitMode, etc.)

### PTY Session Management (`session/`)

`SessionManager` owns all PTY sessions directly (no daemon). Each session:
- Spawns a `claude` process via `portable-pty`
- Runs a reader thread that sends output chunks via MPSC channel
- Maintains a `vt100::Parser` for incremental terminal state
- Stores scrollback (10,000 lines per session)

The main thread drains MPSC channels, feeds data to VT100 parsers, converts parser state to `SessionState`/`ScreenState` for rendering.

### Claude Integration (`claude/`)

- `sessions.rs` — Discovers conversations by parsing `~/.claude/projects/<project-hash>/sessions-index.json` and scanning JSONL files
- `conversation.rs` — Fast status detection by reading last 8KB of JSONL files (Active/WaitingForInput/Idle)
- `grouping.rs` — Groups conversations by Worktree/Directory/Ungrouped with stable ordering during auto-refresh
- `archive.rs` — Soft-delete with separate persistence; archived conversations hidden by default
- `watcher.rs` — Watches sessions-index.json for changes to trigger auto-refresh

### UI Layer (`ui/`)

Ratatui immediate-mode rendering. Key components:
- `sidebar/` — Flattened list of SidebarItems (groups + conversations), inline filtering, archive toggle
- `terminal_pane.rs` — Renders ScreenState cells with colors/attributes, supports text selection and scrollback
- `modal/` — NewProject, Search, Worktree, WorktreeSearch modals with keyboard navigation
- `which_key.rs` — Leader mode command tree popup (Space-triggered, hierarchical)
- `toast.rs` / `toast_widget.rs` — Transient notification system (auto-dismiss, bottom-right)

### Input Handling (`handlers/`, `input/`)

Three input modes: Normal (navigation/commands), Insert (PTY passthrough/modal text entry), Leader (which-key command discovery). Key processing pipeline: global shortcuts → insert mode → leader mode → normal mode.

Notable state machines:
- **Escape sequence** — Detects jk/kj rapid-press (150ms timeout) for exiting insert mode
- **Chord** — "dd" for closing session, numeric prefix for count-based navigation (500ms timeout)

## Code Quality

Strict clippy lints are configured in `Cargo.toml` (`[lints.clippy]`). All warnings are treated as errors in CI. When creating a release, bump the version in `Cargo.toml` to match the release tag.

## Key Patterns

- **Enum dispatch for modals**: `ModalState` enum wraps boxed modal states, with `as_modal_mut()` returning `&mut dyn Modal`
- **MPSC for PTY output**: Reader threads send raw bytes; main thread processes synchronously each tick
- **VT100 → ScreenState conversion**: Parser state is converted to an intermediate `ScreenState` (cached) for rendering
- **Stable group ordering**: `ordered_groups_by_keys` preserves sidebar group order across auto-refresh cycles
- **Project hash discovery**: Absolute path with `/` replaced by `-` maps to `~/.claude/projects/<hash>/`

## Configuration

Config file at `~/.config/claudatui/config.json` (or XDG default). Supports layout options (sidebar position/width) and `dangerous_mode` (skip permission prompts).
