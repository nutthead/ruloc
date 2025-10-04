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
- **Lint**: `cargo clippy`

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
