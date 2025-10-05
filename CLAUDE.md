# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`ruloc` (Rust Lines of Code) is a minimalist CLI tool that counts lines of code in Rust source files. It distinguishes between production code and test code, providing detailed statistics for both.

## Development Commands

- **Build**: `cargo build`
- **Run**: `cargo run -- [args]` (e.g., `cargo run -- -d src/`)
- **Build (release)**: `cargo build --release`
- **Run (release)**: `cargo run --release -- [args]`
- **Test**: `cargo test`
- **Test (single)**: `cargo test <test_name>`
- **Check**: `cargo check` (faster than build, just type-checks)
- **Format**: `cargo fmt`
- **Lint**: `cargo clippy --all-targets --all-features -- -D warnings`
- **Coverage**: `cargo tarpaulin` (requires cargo-tarpaulin installation)

## Code Coverage

The project uses [cargo-tarpaulin](https://github.com/xd009642/tarpaulin) for code coverage analysis.

### Installation

```bash
# Install tarpaulin (requires OpenSSL development libraries)
nix-shell -p openssl pkg-config --run 'cargo install cargo-tarpaulin'

# On Ubuntu/Debian, install dependencies first:
# sudo apt-get install libssl-dev pkg-config

# On Fedora:
# sudo dnf install openssl-devel
```

### Running Coverage

```bash
# Run coverage with default configuration (from .tarpaulin.toml)
cargo tarpaulin

# Coverage must be >= 80% or the command will fail
# Output formats: HTML, XML, JSON, LCOV
# Reports saved to: target/tarpaulin/
```

### Coverage Configuration

Coverage settings are defined in `.tarpaulin.toml`:

- **Minimum coverage**: 80%
- **Output formats**: HTML, XML, JSON, LCOV
- **Engine**: LLVM (more accurate)
- **Output directory**: `target/tarpaulin/`

## Project Structure

Single-file binary crate (all code in `src/main.rs`):

- **Data Structures**: `LineStats`, `FileStats`, `Summary`, `Report` (with serde serialization)
- **CLI**: Argument parsing with clap (derives)
- **Line Analysis**: Classifies lines as blank, comment, or code
- **Production vs Test Detection**: Uses `ra_ap_syntax` AST parsing to identify test modules and functions
- **File Analysis**: Single file or recursive directory traversal
- **Output**: Plain text (default) or JSON format
- **Tests**: Comprehensive unit tests in `#[cfg(test)]` module

## Architecture Notes

- Uses `ra_ap_syntax` for parsing Rust AST to detect `#[test]` and `#[cfg(test)]` attributes
- Separates line classification (blank/comment/code) from production/test classification
- All functionality is in one file for simplicity
- Test detection recursively walks the syntax tree to find test sections
- Line counting uses simple text analysis, while production/test split uses AST parsing

## Hard rules

- **Rule 1:** Run `cargo fmt` after every conversation that changes or adds news `.rs` files
- **Rule 2:** Run `cargo clippy --all-targets --all-features -- -D warnings` after every conversation that changes or adds news `.rs` files
- **Rule 3:** Ensure rustdocs comments are always in-sync with the code they comment
- **Rule 4:** Whenever you add new code to the project, ensure you use Rust patterns that make its all execution paths easily testable
- **Rule 5:** Whenever you add new code to the project, ensure it has complete and eloquent rustdoc comments
- **Rule 6:** Whenever you add new code to the project, ensure it has adequare unit test coverage
- **Rule 7:** Ensure all rustdocs follow the same professional and eloquent style and tone
- **Rule 8:** When you add a dependency to `Cargo.toml`, ensure you are adding the latest stable version of the dependency
- **Rule 9:** Ensure code coverage always remains above 85%
- **Rule 10:** Handover commit tasks to the `/c` SlashCommand

