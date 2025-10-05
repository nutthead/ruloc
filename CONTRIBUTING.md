# Contributing to ruloc

Thank you for your interest in contributing to ruloc! This document provides guidelines and best practices for contributing to the project.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Workflow](#development-workflow)
- [Commit Message Guidelines](#commit-message-guidelines)
- [Pull Request Process](#pull-request-process)
- [Testing](#testing)
- [Code Coverage](#code-coverage)
- [Release Process](#release-process)

## Code of Conduct

This project adheres to a standard code of conduct:

- Be respectful and constructive in all interactions
- Focus on what is best for the community
- Show empathy towards other community members
- Accept constructive criticism gracefully

## Getting Started

### Prerequisites

- Rust 1.90.0 or later
- cargo-tarpaulin (for code coverage)
- cargo-clippy (for linting)

### Setting Up Your Development Environment

```bash
# Clone the repository
git clone https://github.com/nutthead/ruloc.git
cd ruloc

# Build the project
cargo build

# Run tests
cargo test

# Run linting
cargo clippy --all-targets --all-features -- -D warnings

# Check formatting
cargo fmt --all -- --check
```

## Development Workflow

1. **Fork the repository** and create a new branch for your changes
2. **Make your changes** following the coding standards
3. **Write tests** for new functionality
4. **Ensure all tests pass** and coverage remains above 80%
5. **Update documentation** if necessary
6. **Submit a pull request** with a clear description

## Commit Message Guidelines

This project uses [Conventional Commits](https://www.conventionalcommits.org/) with optional footer tags for enhanced changelog generation.

### Commit Message Format

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

### Standard Format (Recommended)

The simplest and recommended format uses conventional commit prefixes:

```bash
feat(cli): add --verbose flag
fix(parser): handle edge case in line counting
docs(readme): update installation instructions
test(stats): add tests for LineStats::merge
refactor(main): simplify argument parsing
```

### Advanced Format with Footer Tags

For fine-grained control over changelog categorization, you can add footer tags:

```bash
feat(cli): add --verbose flag

Implements detailed output mode with file-by-file statistics.

$feat
```

**Available Types:**

| Type | Description | Changelog Group |
|------|-------------|-----------------|
| `feat` | New feature | ⭐ Features |
| `fix` | Bug fix | 🐛 Bug Fixes |
| `docs` | Documentation changes | 📚 Documentation |
| `test` | Test additions or changes | 🧪 Testing |
| `refactor` | Code refactoring | 🔨 Refactor |
| `perf` | Performance improvements | ⚡ Performance |
| `style` | Code style changes | 🎨 Styling |
| `build` | Build system changes | 📦 Build System |
| `ci` | CI/CD changes | 👷 CI/CD |
| `chore` | Miscellaneous changes | 🧹 Miscellaneous |
| `revert` | Revert previous commit | ⏪ Reverts |
| `polish` | Code polish and cleanup | ✨ Polish |

**Common Scopes:**

- `cli` - Command-line interface
- `parser` - Code parsing logic
- `stats` - Statistics calculation
- `io` - Input/output operations
- `tests` - Test infrastructure
- `docs` - Documentation
- `ci` - CI/CD workflows

### Footer Tags (Optional)

Footer tags provide explicit control over changelog generation:

```bash
feat(cli): add JSON output format

Allows exporting statistics in JSON format for programmatic consumption.

$feat
```

**Available Footer Tags:**

- `$feat` - Features
- `$fix` - Bug Fixes
- `$docs` - Documentation
- `$test` - Testing
- `$refactor` - Refactor
- `$perf` - Performance
- `$style` - Styling
- `$build` - Build System
- `$ci` - CI/CD
- `$chore` - Miscellaneous
- `$no-changelog` - Exclude from changelog

### Excluding Commits from Changelog

Use `$no-changelog` for commits that shouldn't appear in the changelog:

```bash
chore(ci): update workflow configuration

Minor workflow adjustments that don't affect users.

$no-changelog
```

### Breaking Changes

Mark breaking changes with `!` and include `BREAKING CHANGE:` in the body:

```bash
feat(cli)!: redesign command-line interface

BREAKING CHANGE: The --directory flag has been renamed to --dir.
All existing scripts using --directory must be updated.

$feat
```

### Examples

**Simple bug fix:**
```bash
fix(parser): handle empty files correctly
```

**Feature with scope:**
```bash
feat(stats): add support for counting test lines
```

**Documentation update (excluded from changelog):**
```bash
docs(readme): fix typo in installation section

$no-changelog
```

**CI change with footer tag:**
```bash
ci(workflows): add code coverage reporting

Integrates Codecov for automatic coverage tracking on PRs.

$ci
```

**Breaking change:**
```bash
feat(api)!: change LineStats field visibility

BREAKING CHANGE: LineStats fields are now private. Use accessor methods instead.

$feat
```

## Pull Request Process

1. **Ensure your code passes all checks:**
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test
   cargo tarpaulin  # Coverage must be ≥ 80%
   ```

2. **Update documentation** if you've changed functionality

3. **Add tests** for new features or bug fixes

4. **Fill out the PR template** with:
   - Clear description of changes
   - Related issue numbers (if applicable)
   - Testing performed
   - Breaking changes (if any)

5. **Respond to review feedback** promptly and professionally

6. **Ensure CI passes** before requesting final review

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Run tests for a specific module
cargo test stats::
```

### Writing Tests

- Place unit tests in the same file as the code being tested
- Use descriptive test names that explain what is being tested
- Test both success and failure cases
- Include edge cases and boundary conditions

Example:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_stats_new_initializes_to_zero() {
        let stats = LineStats::new();
        assert_eq!(stats.total, 0);
        assert_eq!(stats.code, 0);
        assert_eq!(stats.comments, 0);
        assert_eq!(stats.blank, 0);
    }

    #[test]
    fn test_line_stats_add_increments_correctly() {
        let mut stats = LineStats::new();
        stats.add(LineType::Code);
        assert_eq!(stats.code, 1);
        assert_eq!(stats.total, 1);
    }
}
```

## Code Coverage

This project maintains a minimum code coverage of **80%** (target: **85%+**).

### Checking Coverage

```bash
# Install tarpaulin (if not already installed)
cargo install cargo-tarpaulin

# Run coverage
cargo tarpaulin

# Generate HTML report
cargo tarpaulin --out Html
```

### Coverage Reports

- Coverage reports are automatically generated in CI
- PRs that decrease coverage below 80% will fail
- Coverage reports are uploaded to Codecov
- You can view detailed coverage in `target/tarpaulin/tarpaulin-report.html`

## Release Process

Releases are automated using [release-plz](https://release-plz.ieni.dev/):

1. **Merge to master** - Your changes are merged via PR
2. **Automatic release PR** - release-plz creates a release PR with version bump and changelog
3. **Review release PR** - Maintainers review the version bump and changelog
4. **Merge release PR** - Merging creates a git tag
5. **Automated release** - GitHub Actions builds, signs, and publishes the release

### Version Bumping

release-plz follows [Semantic Versioning](https://semver.org/):

- `feat:` commits trigger a **minor** version bump (0.1.0 → 0.2.0)
- `fix:` commits trigger a **patch** version bump (0.1.0 → 0.1.1)
- Breaking changes trigger a **major** version bump (0.1.0 → 1.0.0)

### Changelog Generation

The changelog is automatically generated from commit messages:

- Commits are grouped by type (Features, Bug Fixes, etc.)
- Commits with `$no-changelog` are excluded
- Breaking changes are highlighted prominently
- PR references are preserved for traceability

## Questions or Problems?

- **Bug reports:** Open an issue with a clear description and reproduction steps
- **Feature requests:** Open an issue describing the proposed feature and use case
- **Questions:** Open a discussion or contact the maintainers

Thank you for contributing to ruloc! 🎉
