# ruloc

> A minimalist, AST-driven Rust lines of code counter with intelligent production/test separation.

ruloc provides precise source code metrics for Rust projects by leveraging AST-based parsing to accurately distinguish between production and test code. It counts blank lines, comments, rustdoc documentation, and executable code—all while maintaining a simple, single-file architecture that's easy to understand and extend.

![ruloc](./.readme/ruloc.webp)

## Why ruloc?

- **AST-driven accuracy** — Uses `ra_ap_syntax` for token-level parsing, correctly handling comments in strings, raw strings, and complex macros.
- **Smart test detection** — Automatically identifies `#[test]` functions and `#[cfg(test)]` modules, providing separate metrics for production and test code.
- **Rustdoc-aware** — Distinguishes documentation comments (`///`, `//!`, `/**`, `/*!`) from regular comments.
- **Memory-efficient** — File-backed accumulator supports analyzing arbitrarily large codebases without exhausting RAM.
- **Parallel processing** — Leverages Rayon for concurrent file analysis on multi-core systems.
- **Debug mode** — Line-by-line output with color-coded type markers (PBL, PCO, PCM, PDC, TBL, TCO, TCM, TDC) for detailed inspection.
- **Single file** — Entire implementation in `src/main.rs`. No hidden complexity.

## Quick Start

Analyze your Rust project in seconds:

```sh
# Install from source
cargo install --path .

# Analyze a single file
ruloc --file src/main.rs

# Analyze an entire directory
ruloc --dir src/

# JSON output for CI/CD integration
ruloc --dir . --out-json
```

## Table of Contents

- [Install](#install)
- [Usage](#usage)
- [Output Formats](#output-formats)
- [Debug Mode](#debug-mode)
- [Background](#background)
- [Development](#development)
- [Maintainers](#maintainers)
- [Contributing](#contributing)
- [License](#license)

## Install

### Building from Source

Clone the repository and install locally:

```sh
git clone https://github.com/nutthead/ruloc.git
cd ruloc
cargo install --path .
```

### Using Cargo

If you have Rust installed:

```sh
cargo install ruloc
```

*(Note: Package not yet published to crates.io)*

## Usage

### Basic Analysis

Analyze a single Rust source file:

```sh
ruloc --file src/main.rs
```

Analyze all Rust files in a directory recursively:

```sh
ruloc --dir src/
```

### Output Formats

**Plain text output** (default):

```sh
ruloc --dir src/
```

**JSON output** for programmatic consumption:

```sh
ruloc --dir src/ --out-json
```

### Advanced Options

**Limit maximum file size** to skip large generated files:

```sh
ruloc --dir src/ --max-file-size 1MB
# Supports: bytes (default), KB, MB, GB
# Examples: 1000, 3.5KB, 10MB, 1.1GB
```

**Enable verbose logging** for debugging:

```sh
ruloc --dir src/ --verbose
```

### Debug Mode

Inspect exactly how ruloc classifies each line with debug mode:

```sh
ruloc --file src/main.rs --debug
```

Output shows color-coded prefixes for each line:

```
src/main.rs:
PDC  /// Production rustdoc comment
PCO  fn production() {}
PBL
PCM  // Production comment
TBL
TCO  #[cfg(test)]
TCO  mod tests {
TDC      /// Test rustdoc comment
TCO      #[test]
TCO      fn test_one() {
TCM          // Test comment
TCO      }
TCO  }
```

**Debug marker legend:**
- **PBL** — Production BLank line
- **PCO** — Production COde line
- **PCM** — Production CoMment line
- **PDC** — Production DoC (rustdoc) line
- **TBL** — Test BLank line
- **TCO** — Test COde line
- **TCM** — Test CoMment line
- **TDC** — Test DoC (rustdoc) line

**Disable colors** in debug mode:

```sh
ruloc --file src/main.rs --debug --no-color
```

## Output Formats

### Plain Text

```
Summary:
  Files: 1
  Total:
    All lines: 3580
    Blank lines: 428
    Comment lines: 156
    Rustdoc lines: 812
    Code lines: 2184
  Production:
    All lines: 2068
    Blank lines: 234
    Comment lines: 89
    Rustdoc lines: 812
    Code lines: 933
  Test:
    All lines: 1512
    Blank lines: 194
    Comment lines: 67
    Rustdoc lines: 0
    Code lines: 1251

Files:
  src/main.rs:
    Total:
      All lines: 3580
      Blank lines: 428
      Comment lines: 156
      Rustdoc lines: 812
      Code lines: 2184
    Production:
      All lines: 2068
      Blank lines: 234
      Comment lines: 89
      Rustdoc lines: 812
      Code lines: 933
    Test:
      All lines: 1512
      Blank lines: 194
      Comment lines: 67
      Rustdoc lines: 0
      Code lines: 1251
```

### JSON

```json
{
  "summary": {
    "files": 1,
    "total": {
      "all-lines": 3580,
      "blank-lines": 428,
      "comment-lines": 156,
      "rustdoc-lines": 812,
      "code-lines": 2184
    },
    "production": {
      "all-lines": 2068,
      "blank-lines": 234,
      "comment-lines": 89,
      "rustdoc-lines": 812,
      "code-lines": 933
    },
    "test": {
      "all-lines": 1512,
      "blank-lines": 194,
      "comment-lines": 67,
      "rustdoc-lines": 0,
      "code-lines": 1251
    }
  },
  "files": [
    {
      "path": "src/main.rs",
      "total": { /* ... */ },
      "production": { /* ... */ },
      "test": { /* ... */ }
    }
  ]
}
```

## Background

ruloc was built to provide accurate, production-grade metrics for Rust codebases while maintaining architectural simplicity:

- **Single-file implementation** — All functionality resides in `src/main.rs` (~3600 lines including comprehensive tests and rustdoc), making the codebase transparent and easy to audit.
- **AST-based classification** — Uses the same parser as rust-analyzer (`ra_ap_syntax`) to tokenize source code, ensuring accurate classification even in complex scenarios like comments within raw strings or macro invocations.
- **Two-pass analysis** — First pass classifies each line as blank, comment, rustdoc, or code. Second pass traverses the AST to identify test sections marked with `#[test]` or `#[cfg(test)]` attributes.
- **Scalable architecture** — Implements both in-memory and file-backed accumulators, enabling analysis of projects with millions of lines without memory constraints.
- **Parallel processing** — Uses Rayon to analyze files concurrently, maximizing throughput on multi-core systems.

### How It Works

ruloc employs a dual-analysis strategy:

1. **Token-level line classification:**
   - Parses source into a syntax tree using `ra_ap_syntax`
   - Maps each token to its containing line(s)
   - Classifies lines based on token types (whitespace, comment, rustdoc, code)
   - Handles edge cases: comments in strings, multi-line constructs, raw strings

2. **AST-based test detection:**
   - Recursively traverses the syntax tree
   - Identifies functions with `#[test]` attributes
   - Identifies modules/functions with `#[cfg(test)]` attributes
   - Verifies `cfg(test)` specifically (not `cfg(unix)`, etc.) by inspecting token trees
   - Marks all lines within identified sections as test code

This approach combines the precision of AST parsing with the simplicity of line-based metrics, providing accurate results while remaining conceptually straightforward.

## Development

### Prerequisites

- Rust 1.70+ (uses `let-else` statements)
- Standard Rust toolchain (`rustc`, `cargo`)

### Core Commands

```sh
# Type checking (fast feedback)
cargo check

# Format code
cargo fmt

# Lint with Clippy
cargo clippy --all-targets --all-features -- -D warnings

# Run tests
cargo test

# Build optimized binary
cargo build --release

# Run against a target
cargo run -- --dir src/
```

### Code Coverage

ruloc maintains ≥85% code coverage using [tarpaulin](https://github.com/xd009642/tarpaulin):

```sh
# Install tarpaulin (requires OpenSSL development libraries)
cargo install cargo-tarpaulin

# Run coverage analysis
cargo tarpaulin

# Coverage configuration in .tarpaulin.toml
# Reports: HTML (target/tarpaulin/index.html), XML, JSON, LCOV
```

### Project Structure

```
ruloc/
├── src/
│   └── main.rs          # Complete implementation (~3600 lines)
├── Cargo.toml           # Dependencies and metadata
├── .tarpaulin.toml      # Coverage configuration (≥85% threshold)
├── CLAUDE.md            # Development guidelines for AI assistants
└── README.md            # This file
```

### Contributing Workflow

1. **Format and lint** — Ensure `cargo fmt` and `cargo clippy` pass
2. **Test coverage** — Add tests for new functionality; maintain ≥85% coverage
3. **Documentation** — Update rustdoc comments to match code changes
4. **Commit style** — Follow Conventional Commits (`feat:`, `fix:`, `refactor:`, etc.)

## Maintainers

- [Behrang Saeedzadeh](https://www.behrang.org)

## Contributing

Issues and pull requests are welcome. Before submitting:

1. Run `cargo fmt` and `cargo clippy --all-targets --all-features -- -D warnings`
2. Ensure `cargo test` passes with all tests succeeding
3. Verify `cargo tarpaulin` reports ≥85% coverage
4. Update rustdoc comments for any modified APIs
5. Follow Conventional Commit message format

## License

[MIT](LICENSE)
