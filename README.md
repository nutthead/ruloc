# ruloc

Count lines of Rust code, intelligently separating production from tests.

## Installation

```bash
cargo install --path .
```

## Usage

```bash
# Analyze a single file
ruloc -f src/main.rs

# Analyze all .rs files in a directory (recursive)
ruloc -d src/

# JSON output
ruloc --out-json -d src/

# Enable verbose logging
ruloc --verbose -d src/
```

## Output Formats

### Plain Text (default)

```text
Summary:
  Files: 1
  Total:
    All lines: 710
    Blank lines: 92
    Comment lines: 91
    Code lines: 527
  Production:
    All lines: 530
    Blank lines: 90
    Comment lines: 88
    Code lines: 352
  Test:
    All lines: 180
    Blank lines: 2
    Comment lines: 3
    Code lines: 175

Files:
  src/main.rs:
    Total:
      All lines: 710
      ...
```

### JSON (`--out-json`)

```json
{
  "summary": {
    "files": 1,
    "total": {
      "all-lines": 710,
      "blank-lines": 92,
      "comment-lines": 91,
      "code-lines": 527
    },
    "production": {
      "all-lines": 530,
      "blank-lines": 90,
      "comment-lines": 88,
      "code-lines": 352
    },
    "test": {
      "all-lines": 180,
      "blank-lines": 2,
      "comment-lines": 3,
      "code-lines": 175
    }
  },
  "files": [...]
}
```

## Features

- **Smart test detection**: Uses AST parsing to identify `#[test]` and `#[cfg(test)]` code
- **Accurate line classification**: Distinguishes blank lines, comments, and code
- **Multiple output formats**: Plain text or JSON
- **Fast**: Minimal dependencies, single-purpose tool
- **Single file**: Entire implementation in `src/main.rs` (~710 lines)

## How It Works

ruloc uses two analysis passes:

1. **Line classification**: Simple text analysis identifies blank lines, comments (including `/* */` blocks), and code lines
2. **Production/test split**: AST parsing via `ra_ap_syntax` detects test modules and functions marked with `#[test]` or `#[cfg(test)]` attributes

This dual approach provides accurate metrics while maintaining simplicity.

## Development

```bash
# Build
cargo build

# Test
cargo test

# Run
cargo run -- -d src/
```

## License

See LICENSE file.
