# ruloc

Implement `ruloc` (rust lines of code), which is a minimalist program that counts lines of code in a single `.rs` file, or all `.rs` files under a given directory (recursively):

```bash
# Lines of code in the main.rs file
ruloc -f main.rs

# Lines of code for all .rs files under the src/ directory
ruloc -d src/
```

## Output formats

ruloc can generate the report in multiple formats.

### Default format (plain text)

`$ ruloc [--out-text] -d src/`

```text
Summary:
  Files: 3
  Total:
    All lines: 350
    Blank lines: 30
    Comment lines: 50
    Code lines: 140
  Production:
    All lines: 200
    Blank lines: 20
    Comment lines: 40
    Code lines: 100
  Test:
    All lines: 150
    Blank lines: 10
    Comment lines: 30
    Code lines: 40

Files:
  src/main.rs:
    Total:
      All lines: 250
      Blank lines: 20
      Comment lines: 30
      Code lines: 40
    Production:
      All lines: 120
      Blank lines: 15
      Comment lines: 10
      Code lines: 10
    Test:
      All lines: 130
      Blank lines: 35
      Comment lines: 40
      Code lines: 50
  src/lib.rs:
    Total:
      All lines: 100
      Blank lines: 10
      Comment lines: 20
      Code lines: 100
    Production:
      All lines: 70
      Blank lines: 3
      Comment lines: 10
      Code lines: 57
    Test:
      All lines: 30
      Blank lines: 7
      Comment lines: 10
      Code lines: 43
```

### `json` format (via the `--json` flag)

`$ ruloc --out-json -d src/`

```json
{
  "summary": {
    "files": 3,
    "total": {
      "all-lines": 350,
      "blank-lines": 30,
      "comment-lines": 50,
      "code-lines": 140
    },
    "production": {
      "all-lines": 200,
      "blank-lines": 20,
      "comment-lines": 40,
      "code-lines": 100
    },
    "test": {
      "all-lines": 150,
      "blank-lines": 10,
      "comment-lines": 30,
      "code-lines": 40
    }
  },
  "files": [
    {
      "path": "src/main.rs",
      "total": {
        "all-lines": 250,
        "blank-lines": 20,
        "comment-lines": 30,
        "code-lines": 40
      },
      "production": {
        "all-lines": 120,
        "blank-lines": 15,
        "comment-lines": 10,
        "code-lines": 10
      },
      "test": {
        "all-lines": 130,
        "blank-lines": 35,
        "comment-lines": 40,
        "code-lines": 50
      }
    },
    {
      "path": "src/lib.rs",
      "total": {
        "all-lines": 100,
        "blank-lines": 10,
        "comment-lines": 20,
        "code-lines": 100
      },
      "production": {
        "all-lines": 70,
        "blank-lines": 3,
        "comment-lines": 10,
        "code-lines": 57
      },
      "test": {
        "all-lines": 30,
        "blank-lines": 7,
        "comment-lines": 10,
        "code-lines": 43
      }
    }
  ]
}
```

## CLI

The CLI should support the following options:

- `-f, --file <FILE>`: Specify a single `.rs` file to analyze.
- `-d, --dir <DIR>`: Specify a directory to analyze all `.rs` files recursively.
- `--out-text`: Output the report in plain text format (default).
- `--out-json`: Output the report in JSON format.
- `-h, --help`: Show help information.
- `-V, --version`: Show version information.
- `--verbose`: Enable verbose output for debugging purposes.

If the program is run without any arguments, it should display the help information.

## Implementation details

- Use the `clap` crate for command-line argument parsing.
- Use the `serde` and `serde_json` crates for JSON serialization.
- Use the `walkdir` crate for recursive directory traversal.
- Use the `ra_ap_syntax` crate for parsing Rust source files.
- The entire implementation (including unit tests) should be contained within a single `main.rs` file.
- All code should have eloquent, meaningful rustdoc comments.
  - Ultrathink and write comments according to best practices.
- Ultrathink and optimize for clarity and conciseness.
- Ultrathink and search for testability patterns in Rust and write the code to be easily testable.
- Ultrathink and handle edge cases, such as empty files, files with only comments, etc.
- Ultrathink and ensure the program handles large codebases efficiently.
- Ultrathink and ensure the entire impelemtation is following the same coding style and conventions.
- Ensure functions are small and focused on a single task.
- Break down complex logic into smaller, reusable functions.
- Honor DRY (Don't Repeat Yourself) and SOLID principles.
- Use traits and generics where appropriate to enhance code testability and organization.
- This is a command-line tool, so ensure it doesn't have a `lib.rs` file.
- Specify the `bin` target in `Cargo.toml` to indicate that this is a binary crate.
- Add trace-level logging using the `log` crate, which should be enabled with the `--verbose` flag.
