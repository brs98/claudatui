# Contributing to claudatui

Thanks for your interest in contributing!

## Quick Start

1. **Fork** this repository
2. **Clone** your fork: `git clone https://github.com/YOUR_USERNAME/claudatui.git`
3. **Create a branch**: `git checkout -b my-feature`
4. **Make your changes**
5. **Ensure CI passes locally**:
   ```bash
   cargo fmt --check
   cargo clippy -- -D warnings
   cargo test
   ```
6. **Commit** your changes with a clear message
7. **Push** to your fork and open a **Pull Request**

## Guidelines

- **Don't modify the version** in `Cargo.toml` — releases are handled by maintainers
- **Run `cargo fmt`** before committing to ensure consistent formatting
- **Add tests** for new functionality when possible
- **Keep PRs focused** — one feature or fix per PR

## What Happens Next

1. CI automatically runs on your PR
2. A maintainer will review your changes
3. Once approved and merged, your contribution will be included in the next release

## Development

```bash
# Run the app
cargo run

# Run with debug logging
RUST_LOG=debug cargo run

# Run tests
cargo test

# Check formatting
cargo fmt --check

# Run linter
cargo clippy -- -D warnings
```

## Questions?

Open an issue if you have questions or want to discuss a feature before implementing it.
