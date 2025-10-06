# CLAUDE.md

This file provides comprehensive guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`ruloc` (Rust Lines of Code) is a production-grade, AST-driven CLI tool that counts lines of code in Rust source files with intelligent production/test separation.

### Key Features

- **AST-based parsing**: Uses `ra_ap_syntax` (rust-analyzer's parser) for token-level accuracy
- **Smart test detection**: Automatically identifies `#[test]` functions and `#[cfg(test)]` modules
- **Rustdoc awareness**: Distinguishes documentation comments (`///`, `//!`) from regular comments
- **Parallel processing**: Leverages Rayon for concurrent file analysis across multiple cores
- **Memory-efficient**: File-backed accumulator supports analyzing arbitrarily large codebases
- **Debug mode**: Line-by-line output with color-coded markers (PBL, PCO, PCM, PDC, TBL, TCO, TCM, TDC)
- **File size limits**: Skip large generated files with configurable size thresholds
- **Progress tracking**: Visual progress bar for directory analysis
- **Dual output formats**: Plain text (human-readable) and JSON (machine-readable)

## Project Architecture

### Single-file Design

All code resides in `src/main.rs` (~4600 lines including comprehensive tests and rustdoc). This deliberate choice prioritizes:

- **Transparency**: Easy to audit and understand the entire codebase
- **Simplicity**: No hidden complexity across multiple modules
- **Maintainability**: Changes are localized and easy to track

### Core Data Structures

```rust
// Line classification
enum LineType { Blank, Comment, Rustdoc, Code }

// Statistics tracking
struct LineStats {
    all_lines: usize,
    blank_lines: usize,
    comment_lines: usize,
    rustdoc_lines: usize,
    code_lines: usize,
}

// File-level statistics
struct FileStats {
    path: String,
    total: LineStats,
    production: LineStats,
    test: LineStats,
}

// Accumulator pattern for memory efficiency
trait StatsAccumulator {
    fn add_file(&mut self, stats: &FileStats) -> Result<(), String>;
    fn get_summary(&self) -> Summary;
    fn iter_files(&self) -> Result<Box<dyn Iterator<Item = FileStats> + '_>, String>;
    fn flush(&mut self) -> Result<(), String>;
}

// Two implementations:
struct InMemoryAccumulator { ... }      // Fast, memory-bound
struct FileBackedAccumulator { ... }    // Scalable, disk-backed
```

### Analysis Pipeline

1. **File Discovery** (parallel via Rayon)
   - Recursively walk directories
   - Filter for `.rs` files
   - Apply size limits if configured

2. **Line Classification** (token-based)
   - Parse source into syntax tree using `ra_ap_syntax`
   - Map byte offsets to line numbers
   - Classify each line based on token types
   - Handle edge cases: comments in strings, multi-line constructs

3. **Test Detection** (AST traversal)
   - Recursively traverse syntax tree
   - Identify `#[test]` functions
   - Identify `#[cfg(test)]` modules/functions
   - Verify `cfg(test)` specifically (not `cfg(unix)`, etc.)
   - Mark all lines within test sections

4. **Accumulation** (streaming)
   - Stream stats to accumulator (in-memory or file-backed)
   - Generate summary statistics
   - Support iteration over file stats

5. **Output** (formatted)
   - Plain text: Human-readable summary + per-file breakdown
   - JSON: Machine-readable for CI/CD integration

### Memory Management

The project uses an **accumulator pattern** for memory-efficient processing:

- **InMemoryAccumulator**: Fast, stores all stats in `Vec<FileStats>`
  - Use for: Small to medium codebases (< 10K files)
  - Advantage: Direct access, no I/O overhead

- **FileBackedAccumulator**: Scalable, streams stats to temporary file
  - Use for: Large codebases (10K+ files)
  - Advantage: Constant memory usage regardless of project size
  - Implementation: JSON Lines format with buffered I/O

## Development Commands

### Build & Run

```bash
# Development build
cargo build

# Run with arguments
cargo run -- --dir src/

# Release build (optimized)
cargo build --release

# Run release binary
cargo run --release -- --dir src/
```

### Testing

```bash
# Run all tests (158 tests)
cargo test

# Run specific test
cargo test test_classify_lines

# Run with output
cargo test -- --nocapture

# Run tests for specific pattern
cargo test accumulator
```

### Code Quality

```bash
# Fast type checking
cargo check

# Format code (auto-fix)
cargo fmt

# Format check (CI)
cargo fmt --all -- --check

# Lint with Clippy
cargo clippy --all-targets --all-features -- -D warnings

# Documentation check
cargo doc --no-deps --all-features
```

### Code Coverage

```bash
# Install tarpaulin (one-time setup)
cargo install cargo-tarpaulin

# Run coverage analysis
cargo tarpaulin

# Generate HTML report
cargo tarpaulin --out Html

# Open HTML report
open target/tarpaulin/index.html  # macOS
xdg-open target/tarpaulin/index.html  # Linux
```

**Coverage Requirements:**
- Minimum: **70%** (enforced in CI via `.tarpaulin.toml`)
- Target: Maintain or improve current coverage
- Reports: HTML, XML, JSON, LCOV formats in `target/tarpaulin/`

## CLI Usage Patterns

### Basic Analysis

```bash
# Analyze single file
ruloc --file src/main.rs

# Analyze directory
ruloc --dir src/

# Analyze with JSON output
ruloc --dir . --out-json
```

### Advanced Features

```bash
# Debug mode (line-by-line breakdown)
ruloc --file src/main.rs --debug

# Debug without colors
ruloc --file src/main.rs --debug --no-color

# File size limit (skip large files)
ruloc --dir src/ --max-file-size 1MB

# Verbose logging
ruloc --dir src/ --verbose
```

### Debug Mode Markers

- **PBL**: Production Blank Line
- **PCO**: Production Code Line
- **PCM**: Production Comment Line
- **PDC**: Production Documentation Line
- **TBL**: Test Blank Line
- **TCO**: Test Code Line
- **TCM**: Test Comment Line
- **TDC**: Test Documentation Line

## Common Development Workflows

### Adding a New Feature

1. **Plan**: Document the feature in code comments
2. **Implement**: Add code with rustdoc comments
3. **Test**: Write comprehensive unit tests
4. **Format**: Run `cargo fmt`
5. **Lint**: Run `cargo clippy --all-targets --all-features -- -D warnings`
6. **Coverage**: Ensure `cargo tarpaulin` stays ≥ 70%
7. **Commit**: Use `/c` slash command for conventional commits

### Fixing a Bug

1. **Reproduce**: Write a failing test that demonstrates the bug
2. **Fix**: Implement the fix
3. **Verify**: Ensure the test passes
4. **Regression**: Check other tests still pass
5. **Format**: Run `cargo fmt`
6. **Lint**: Run `cargo clippy --all-targets --all-features -- -D warnings`
7. **Commit**: Use `/c` slash command

### Refactoring

1. **Tests first**: Ensure comprehensive test coverage exists
2. **Refactor**: Make changes while keeping tests green
3. **Document**: Update rustdoc comments
4. **Format**: Run `cargo fmt`
5. **Lint**: Run `cargo clippy --all-targets --all-features -- -D warnings`
6. **Coverage**: Verify coverage doesn't decrease
7. **Commit**: Use `/c` slash command

## Troubleshooting

### Compilation Errors

```bash
# Clean build artifacts
cargo clean

# Rebuild from scratch
cargo build

# Check for syntax errors only
cargo check
```

### Test Failures

```bash
# Run specific failing test
cargo test test_name -- --nocapture

# Run with backtrace
RUST_BACKTRACE=1 cargo test

# Run in release mode (sometimes reveals issues)
cargo test --release
```

### Coverage Issues

```bash
# Clean coverage artifacts
rm -rf target/tarpaulin/

# Run coverage with verbose output
cargo tarpaulin --verbose

# Check .tarpaulin.toml configuration
cat .tarpaulin.toml
```

### Clippy Warnings

```bash
# Fix auto-fixable issues
cargo clippy --fix --all-targets --all-features

# Explain a specific lint
rustc --explain E0308
```

## Dependencies

Current dependencies (from `Cargo.toml`):

- **clap** (4.5.48): CLI argument parsing with derives
- **serde** (1.0.228): Serialization framework
- **serde_json** (1.0.145): JSON serialization
- **walkdir** (2.5.0): Recursive directory traversal
- **ra_ap_syntax** (0.0.301): Rust AST parsing (from rust-analyzer)
- **log** (0.4.28): Logging facade
- **env_logger** (0.11.8): Logger implementation
- **rayon** (1.11.0): Data parallelism
- **indicatif** (0.18.0): Progress bars
- **tempfile** (3.14.0): Temporary file handling
- **colored** (3.0.0): Terminal color output

## Soft Rules

Guidelines that should be followed when reasonable:

1. **Streaming over buffering**: Prefer streaming data to files over large in-memory structures
2. **Functional style**: Use iterators and functional patterns where they improve clarity
3. **Early returns**: Use early returns to reduce nesting
4. **Error context**: Provide helpful error messages with actionable guidance
5. **Performance awareness**: Profile before optimizing, but be mindful of algorithmic complexity

## Hard Rules

**MUST be followed** - these are non-negotiable:

1. **Formatting**: Run `cargo fmt` after every conversation that changes or adds new `.rs` files
2. **Linting**: Run `cargo clippy --all-targets --all-features -- -D warnings` after every conversation that changes or adds new `.rs` files
3. **Documentation sync**: Ensure rustdoc comments are always in sync with the code they document
4. **Testability**: Use Rust patterns that make all execution paths easily testable
5. **Documentation quality**: New code must have complete and eloquent rustdoc comments
6. **Test coverage**: New code must have adequate unit test coverage
7. **Documentation style**: All rustdocs must follow the same professional and eloquent style and tone
8. **Dependency versions**: When adding a dependency to `Cargo.toml`, use the latest stable version
9. **Coverage threshold**: Ensure code coverage always remains above 70%
10. **Commit workflow**: Handover commit tasks to the `/c` SlashCommand

## Testing Strategy

### Unit Test Organization

All tests are in `#[cfg(test)] mod tests` at the end of `main.rs`:

- **Data structure tests**: LineStats, FileStats, Summary, Report
- **Line classification tests**: Blank, comment, rustdoc, code detection
- **Test detection tests**: `#[test]` and `#[cfg(test)]` identification
- **Accumulator tests**: In-memory and file-backed implementations
- **Integration tests**: Full file and directory analysis workflows
- **Error handling tests**: Edge cases and failure scenarios
- **Output tests**: Text and JSON formatting

### Coverage Areas

- Line classification edge cases (comments in strings, raw strings)
- AST traversal for test detection
- File size limit enforcement
- Parallel processing correctness
- Accumulator error handling
- File I/O error scenarios
- JSON serialization/deserialization

### Test Naming Convention

```rust
#[test]
fn test_<component>_<scenario>_<expected_result>() {
    // Example: test_classify_lines_with_rustdoc_correctly_identified
}
```

## Project Metadata

- **Name**: ruloc
- **Version**: 0.1.1
- **Rust Edition**: 2024
- **Minimum Rust**: 1.90.0
- **License**: MIT
- **Repository**: https://github.com/nutthead/ruloc
- **Lines of Code**: ~4600 (including tests and docs)

## Key Implementation Details

### Why Single File?

The entire implementation fits in one file because:
1. **Clarity**: All code is immediately visible
2. **Simplicity**: No module resolution or import complexity
3. **Auditability**: Easy to review the entire implementation
4. **Teaching**: Serves as a reference for Rust patterns

### Why Two Accumulators?

- **InMemoryAccumulator**: Optimal for typical use (fast, simple)
- **FileBackedAccumulator**: Handles extreme cases (millions of files)
- **Trait abstraction**: Allows swapping implementations transparently

### Why AST Parsing?

String-based line counting fails on:
- Comments inside raw strings: `r#"// not a comment"#`
- Multi-line comments: `/* spans\nmultiple\nlines */`
- Macro-generated code: `println!("// not a comment")`

AST parsing provides token-level accuracy.

### Why Rayon for Parallelism?

- Data parallelism is the bottleneck (many files to analyze)
- Rayon provides automatic work-stealing thread pool
- Minimal code: `.par_iter()` instead of `.iter()`
- Excellent performance on multi-core systems

## Release Process

Releases are automated via `release-plz`:

1. Merge PR to `master`
2. release-plz creates release PR with version bump + changelog
3. Maintainer reviews and merges release PR
4. GitHub Actions builds, signs, and publishes the release

**Version Bumping** (Semantic Versioning):
- `feat:` commits → minor bump (0.1.0 → 0.2.0)
- `fix:` commits → patch bump (0.1.0 → 0.1.1)
- Breaking changes → major bump (0.1.0 → 1.0.0)

## Additional Resources

- **README.md**: User-facing documentation
- **CONTRIBUTING.md**: Contribution guidelines and commit conventions
- **.tarpaulin.toml**: Coverage configuration
- **.github/workflows/**: CI/CD pipelines
- **clippy.toml**: Clippy configuration (cognitive complexity threshold)

## Soft rules

- **Rule 1:** When implementing code, prefer iterating over collections without collecting the results and assigning it to a variable
- **Rule 2:** Whenever new code is added to project, write unit tests that cover all its execution paths and control flows

## Hard rules

- **Rule 1:** Run `cargo fmt` after every conversation that changes or adds news `.rs` files
- **Rule 2:** Run `cargo clippy --all-targets --all-features -- -D warnings` after every conversation that changes or adds news `.rs` files 
- **Rule 3:** Ensure rustdoc comments are always in-sync with the code they comment
- **Rule 4:** Whenever you add new code to the project, ensure you use Rust patterns that make all execution paths of the new code easily testable
- **Rule 5:** Whenever you add new code to the project, ensure it has complete and eloquent rustdoc comments
- **Rule 6:** Ensure all rustdoc comments follow a consistent, professional, and eloquent style and tone
- **Rule 7:** When you add a dependency to `Cargo.toml`, ensure you are adding the latest stable version of the dependency
- **Rule 8:** Ensure code coverage always remains 1% above the limit configured in tarpaulin
- **Rule 9:** Handover commit tasks to the `/c` SlashCommand

