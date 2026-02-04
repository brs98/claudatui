# claudatui

A TUI (Terminal User Interface) for browsing and managing Claude Code conversations.

## Features

- Browse and navigate Claude Code conversation history
- View conversation details in a terminal-friendly interface
- File explorer integration
- Clipboard support for copying content
- Real-time file watching for conversation updates

## Installation

### Prerequisites

- Rust 1.88+ (install via [rustup](https://rustup.rs/))
- A working [Claude Code](https://claude.ai/claude-code) installation

To check your Rust version:
```bash
rustc --version
```

To update Rust:
```bash
rustup update
```

### From Source

```bash
# Clone the repository
git clone https://github.com/brs98/claudatui.git
cd claudatui

# Build and install
cargo install --path .
```

### Using Cargo

```bash
cargo install --git https://github.com/brs98/claudatui.git
```

## Usage

```bash
claudatui
```

## Development

### Setup

```bash
# Clone the repository
git clone https://github.com/brs98/claudatui.git
cd claudatui

# Build the project
cargo build

# Run in development mode
cargo run
```

### Project Structure

```
src/
├── main.rs          # Entry point
├── app.rs           # Application state and logic
├── event.rs         # Event handling
├── lib.rs           # Library exports
├── bin/             # Binary utilities
├── claude/          # Claude Code integration
├── pty/             # Pseudo-terminal handling
├── session/         # Session management
└── ui/              # UI components (ratatui)
```

### Building

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run
```

### Dependencies

Key dependencies used in this project:

- **ratatui** - Terminal UI framework
- **crossterm** - Cross-platform terminal manipulation
- **tokio** - Async runtime
- **serde/serde_json** - Serialization
- **notify** - File system notifications
- **portable-pty** - Pseudo-terminal support
- **arboard** - Clipboard access

## License

MIT
