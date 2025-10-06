---
description: Create a high-quality GitHub pull request with intelligent branch handling and comprehensive PR details
allowed-tools: Bash, Read, Edit
---

# Create High-Quality Pull Request for ruloc

Create a professional GitHub pull request tailored for the ruloc project, analyzing changes and generating comprehensive PR content.

## Workflow

### 1. Branch Assessment and Preparation

**Check current branch:**
```bash
git branch --show-current
```

**If on master branch:**
- Analyze the staged or recent changes:
  ```bash
  git status
  git diff --cached
  git log -1 --oneline
  ```
- Generate an appropriate branch name based on the changes:
    - Use conventional prefixes: `feat/`, `fix/`, `refactor/`, `docs/`, `chore/`, `perf/`, `test/`, `ci/`
    - Create descriptive, kebab-case names (e.g., `feat/json-output`, `fix/test-detection`, `perf/memory-optimization`)
- Create and checkout the new branch:
  ```bash
  git checkout -b <generated-branch-name>
  ```

**If already on a feature branch:**
- Proceed with the current branch

### 2. Analyze Changes

**Gather comprehensive information about the changes:**

```bash
# Get the list of changed files
git diff master...HEAD --name-status

# Get detailed diff
git diff master...HEAD

# Get commit messages
git log master..HEAD --oneline

# Get detailed commit information
git log master..HEAD --pretty=format:"%h - %s%n%b"
```

### 3. Generate PR Content

**Analyze the gathered information and create:**

**Title:**
- Follow conventional commit format: `type(scope): description`
- Types: `feat`, `fix`, `refactor`, `docs`, `style`, `test`, `chore`, `perf`, `ci`, `build`
- Common scopes: `cli`, `parser`, `stats`, `io`, `tests`, `docs`, `ci`, `analysis`
- Keep it concise (50-72 characters preferred)
- Use imperative mood (e.g., "Add feature" not "Added feature")

**Body:**
Structure the PR description matching the project's style (with emojis and detailed sections):

```markdown
## Summary

[2-4 sentence overview of what this PR accomplishes, including:]
- **[Category]**: Brief description
- **[Category]**: Brief description
- **[Category]**: Brief description

## Key Changes

### 🚀 [Primary Change Category]

**[Specific change title]** (src/main.rs:[line-range])
- Detailed explanation of what changed
- Why this change was made
- Impact on functionality or performance
- Technical details (e.g., memory savings, algorithmic improvements)

### 🔧 [Secondary Change Category]

**[Specific change title]** (src/main.rs:[line-range])
- Detailed explanation
- Implementation notes
- Benefits

### 📝 [Additional Category if needed]

**[Specific change title]** (file-path:line-range)
- Details about the change

## Testing

**All quality checks passed:**
- ✅ `cargo fmt --all -- --check` - Code formatting verified
- ✅ `cargo clippy --all-targets --all-features -- -D warnings` - No linting issues
- ✅ `cargo test` - All 158 tests pass
- ✅ `cargo tarpaulin` - Coverage maintained at ≥70%

**Test coverage details:**
- [List new tests added or modified]
- [Coverage percentage if changed]

## CI/CD Status

This PR will trigger the following CI checks:
- **Quick checks**: fmt, clippy, documentation
- **Security audit**: cargo-audit, cargo-deny
- **Unit tests**: Linux, macOS (ARM), Windows, Linux (musl)
- **Coverage**: Tarpaulin with automatic PR comment

## Type of Change

- [ ] 🐛 Bug fix (non-breaking change which fixes an issue)
- [ ] ⭐ New feature (non-breaking change which adds functionality)
- [ ] 💥 Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] 📚 Documentation update
- [ ] 🔨 Refactoring (no functional changes)
- [ ] ⚡ Performance improvement
- [ ] 🧪 Testing improvements
- [ ] 🎨 Code style/formatting
- [ ] 🧹 Miscellaneous/chore

## Related Issues

[Reference any related issues: Fixes #123, Closes #456, Related to #789]

## Breaking Changes

[If this is a breaking change, describe:]
- What breaks
- Migration path for users
- Why the breaking change is necessary

## Release Impact

**Version bump**: [patch | minor | major]
**Changelog category**: [Features | Bug Fixes | Documentation | etc.]
**Footer tags used**: [List any `$feat`, `$fix`, `$no-changelog` tags from commits]

This PR will be included in the next release created by release-plz.

## Pre-merge Checklist

- [ ] All commits follow conventional commit format (verified via `/c` command)
- [ ] Code formatted with `cargo fmt --all`
- [ ] Clippy passes with `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] All tests pass with `cargo test`
- [ ] Coverage ≥70% maintained with `cargo tarpaulin`
- [ ] Documentation (rustdoc comments) updated where needed
- [ ] Self-review completed
- [ ] CI checks passing (will be verified by GitHub Actions)

## Additional Context

[Any additional information that reviewers should know:]
- Implementation approach and alternatives considered
- Performance benchmarks or measurements
- Screenshots or examples (if applicable)
- Migration notes
- Dependencies added/updated/removed
```

### 4. Create the Pull Request

**Ensure branch is pushed:**
```bash
# Push current branch to origin
git push -u origin HEAD
```

**Create the PR using GitHub CLI:**
```bash
gh pr create \
  --base master \
  --head <current-branch> \
  --title "<generated-title>" \
  --body "<generated-body>"
```

**Alternative interactive mode (if you need to make edits):**
```bash
gh pr create --base master --head <current-branch> --web
```

### 5. Verification and Output

**After creating the PR:**
1. Display the PR URL
2. Show a summary of what was created
3. Remind about CI checks that will run:
   ```
   ✅ PR created successfully!

   The following CI checks will run automatically:
   - Quick checks (fmt, clippy, docs)
   - Security audit
   - Unit tests (4 platforms)
   - Code coverage (with PR comment)

   Monitor the checks at: <PR_URL>
   ```

## Best Practices for ruloc

1. **Ensure all quality checks pass locally** before creating PR:
   ```bash
   cargo fmt --all -- --check && \
   cargo clippy --all-targets --all-features -- -D warnings && \
   cargo test && \
   cargo tarpaulin
   ```

2. **Use conventional commits** - All commits should have been created via the `/c` command

3. **Include line references** - When describing changes, include `src/main.rs:line-range` references

4. **Single-file context** - Remember that all code is in `src/main.rs` (except tests)

5. **Coverage awareness** - Ensure new code is tested and coverage stays ≥70%

6. **Keep PRs focused** - One logical change per PR for easier review and cleaner git history

7. **Document performance impacts** - If changes affect performance, include measurements

8. **Link to issues** - Use GitHub keywords (Fixes, Closes, Resolves) to auto-close issues

9. **Add appropriate emojis** - Match the project's style with emoji categories (🚀, 🔧, 📝, etc.)

10. **Consider release-plz** - Your commits will be grouped by type in the changelog

## Error Handling

- If not in a git repository, inform the user
- If no changes exist, inform the user to commit first (suggest using `/c`)
- If `gh` is not authenticated, prompt: `gh auth login`
- If the branch already has an open PR, show the existing PR: `gh pr list --head <branch>`
- If on master branch with uncommitted changes, warn about creating a feature branch first

## Notes

- **Base branch**: Always use `master` (this project does not use `main`)
- **Commit format**: Follows conventional commits with footer tags (`$feat`, `$fix`, `$no-changelog`)
- **Coverage requirement**: Minimum 70% (enforced by CI)
- **Test count**: Currently 158 tests (mention if this changes)
- **Release process**: PRs merged to master trigger release-plz to create release PRs
- **CI platform**: GitHub Actions with caching and parallel execution
- **Required tools**: GitHub CLI (`gh`) must be installed and authenticated