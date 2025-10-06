# Release Process

This document provides a comprehensive guide to the automated release process for ruloc, detailing how releases are created, published, and distributed.

## Table of Contents

- [Overview](#overview)
- [Release Automation Architecture](#release-automation-architecture)
- [The Release Pipeline](#the-release-pipeline)
- [Developer Workflow](#developer-workflow)
- [Manual Intervention Points](#manual-intervention-points)
- [Repository Configuration](#repository-configuration)
- [Troubleshooting](#troubleshooting)
- [Security and Attestation](#security-and-attestation)
- [Advanced Topics](#advanced-topics)

## Overview

ruloc employs a fully automated release system built on three key technologies:

- **[release-plz](https://release-plz.ieni.dev/)** - Automated release management for Rust projects
- **[GitHub Actions](https://docs.github.com/en/actions)** - CI/CD automation platform
- **[Conventional Commits](https://www.conventionalcommits.org/)** - Standardized commit message format

This system automates version bumping, changelog generation, binary building, artifact signing, and publication to both GitHub Releases and crates.io.

### Key Features

- **Zero Manual Version Management** - Versions are calculated from commit history
- **Automatic Changelog Generation** - Changelogs are built from conventional commits
- **Comprehensive Binary Distribution** - Supports 9 platform targets
- **Cryptographic Signing** - All artifacts signed with Sigstore/Cosign
- **SLSA Level 3 Provenance** - Build attestations for supply chain security
- **Reproducible Builds** - Locked dependencies ensure consistency

## Release Automation Architecture

The release system consists of three coordinated GitHub Actions workflows:

```
┌─────────────────────────────────────────────────────────────────────┐
│                        RELEASE PIPELINE                             │
└─────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│ 1. RELEASE PR WORKFLOW (.github/workflows/release-pr.yml)            │
├──────────────────────────────────────────────────────────────────────┤
│ Trigger:  Push to master (code/dependency changes)                   │
│ Token:    NH_RELEASE_PLZ_TOKEN (Fine-grained PAT)                    │
│ Purpose:  Analyze commits → Calculate version bump → Create PR       │
│ Output:   Release PR with version bump and changelog                 │
└──────────────────────────────────────────────────────────────────────┘
                                    ↓
                        (Maintainer reviews and merges)
                                    ↓
┌──────────────────────────────────────────────────────────────────────┐
│ 2. RELEASE TAG WORKFLOW (.github/workflows/release-plz.yml)          │
├──────────────────────────────────────────────────────────────────────┤
│ Trigger:  Push to master (version change in Cargo.toml)              │
│ Token:    GITHUB_TOKEN (Automatic)                                   │
│ Purpose:  Detect version change → Create git tag                     │
│ Output:   Git tag (v*.*.*)                                           │
└──────────────────────────────────────────────────────────────────────┘
                                    ↓
                        (Tag triggers release workflow)
                                    ↓
┌──────────────────────────────────────────────────────────────────────┐
│ 3. RELEASE WORKFLOW (.github/workflows/release.yml)                  │
├──────────────────────────────────────────────────────────────────────┤
│ Trigger:  Tag push (v[0-9]+.[0-9]+.[0-9]+)                           │
│ Token:    GITHUB_TOKEN + CARGO_REGISTRY_TOKEN                        │
│ Purpose:  Build → Sign → Publish → Verify                            │
│ Output:   GitHub Release + crates.io publication                     │
└──────────────────────────────────────────────────────────────────────┘

                    ↓                           ↓
        ┌──────────────────┐       ┌──────────────────────┐
        │ GitHub Releases  │       │     crates.io        │
        │ • Binaries       │       │ • Source package     │
        │ • Signatures     │       │ • Documentation      │
        │ • SBOM           │       │                      │
        │ • Attestations   │       │                      │
        └──────────────────┘       └──────────────────────┘
```

## The Release Pipeline

### Stage 1: Release PR Creation

**When:** After any code changes are merged to `master`

**What Happens:**

1. **Workflow Trigger**
   - Detects changes to `src/`, `Cargo.toml`, `Cargo.lock`, or workflow files
   - Skips if commit contains `[skip ci]`

2. **Version Calculation**
   - Analyzes all commits since last release
   - Applies Semantic Versioning rules:
     - `feat:` → Minor bump (0.1.0 → 0.2.0)
     - `fix:` → Patch bump (0.1.0 → 0.1.1)
     - Breaking changes → Major bump (0.1.0 → 1.0.0)
   - Uses `cargo-semver-checks` to detect API breaking changes

3. **Changelog Generation**
   - Groups commits by type (Features, Bug Fixes, etc.)
   - Excludes commits tagged with `$no-changelog`
   - Preserves PR references for traceability
   - Applies conventional commit prefixes and footer tags

4. **Release PR Creation**
   - Creates PR with updated `Cargo.toml` version
   - Includes generated `CHANGELOG.md` section
   - Provides version bump summary and checklist
   - Uses `NH_RELEASE_PLZ_TOKEN` to trigger CI on the PR

**Example Release PR:**

```markdown
## 🚀 Release PR for v0.2.0

This PR was automatically created by release-plz and contains:

### 📦 Version Updates
- **ruloc**: `0.1.1` → `0.2.0`

### 📝 Changelog for ruloc
## [0.2.0] - 2025-10-06

### ⭐ Features
- Add JSON output format (#15)
- Support recursive directory scanning (#14)

### 🐛 Bug Fixes
- Handle empty files correctly (#13)

### ✅ Checklist
- [ ] Version bump looks correct
- [ ] Changelog entries are accurate
- [ ] All CI checks pass
- [ ] Breaking changes are properly documented

### 🔄 Merge Instructions
When you merge this PR, the following will happen automatically:
1. New git tag `v0.2.0` will be created
2. GitHub release will be published with binaries
3. The crate will be published to crates.io
4. Attestations and signatures will be generated
```

### Stage 2: Tag Creation

**When:** Release PR is merged to `master`

**What Happens:**

1. **Version Change Detection**
   - Compares `HEAD~1:Cargo.toml` with current `Cargo.toml`
   - Extracts version strings using `cargo metadata`
   - Handles edge cases (first commit, no changes)

2. **Tag Creation**
   - Creates annotated git tag: `v{VERSION}`
   - Tag pattern: `v[0-9]+.[0-9]+.[0-9]+`
   - Uses `GITHUB_TOKEN` (doesn't trigger workflows)
   - Executes `release-plz release` command

**Example:**

```bash
# When release PR merges with version change 0.1.1 → 0.2.0
git tag v0.2.0
git push origin v0.2.0
```

### Stage 3: Build and Publish

**When:** Git tag matching `v[0-9]+.[0-9]+.[0-9]+` is pushed

**What Happens:**

1. **Preparation** (Job: `prepare-release`)
   - Extracts version from tag
   - Validates version format
   - Verifies `Cargo.toml` version matches tag

2. **Security Scanning** (Job: `security-scan`)
   - Runs `cargo audit` for known vulnerabilities
   - Checks license compliance with `cargo deny`
   - Generates SBOM (CycloneDX format, JSON + XML)

3. **Binary Building** (Job: `build-binaries`)
   - Builds optimized binaries for 9 platforms:
     - Linux: x86_64-gnu, x86_64-musl, aarch64-gnu, aarch64-musl, armv7
     - macOS: aarch64 (Apple Silicon)
     - Windows: x86_64, aarch64 (experimental)
     - RISC-V: riscv64gc (experimental)
   - Uses `cross` for cross-compilation
   - Applies aggressive optimizations (LTO, single codegen unit)
   - Creates platform-specific archives (.tar.gz for Unix, .zip for Windows)
   - Generates SHA256 checksums

4. **Attestation & Signing** (Job: `attestation`)
   - Generates SLSA Build Level 3 provenance
   - Signs artifacts with Sigstore/Cosign (keyless signing)
   - Creates `.sig` and `.crt` files for each binary
   - Uses GitHub's OIDC token for identity verification

5. **Changelog Generation** (Job: `generate-changelog`)
   - Generates release notes using `git-cliff`
   - Includes version comparison links
   - Adds verification instructions

6. **GitHub Release** (Job: `publish-release`)
   - Creates GitHub release for the tag
   - Uploads all binaries and archives
   - Attaches signatures and certificates
   - Includes SBOM and security scan results
   - Publishes release notes

7. **crates.io Publication** (Job: `publish-crate`)
   - Validates `CARGO_REGISTRY_TOKEN` is configured
   - Verifies package builds correctly
   - Publishes to crates.io with `cargo publish`
   - Can be skipped via `skip_publish` workflow input

8. **Release Verification** (Job: `verify-release`)
   - Downloads x86_64 Linux binary from GitHub
   - Verifies Cosign signature
   - Polls crates.io for publication (exponential backoff)
   - Validates crate metadata

**Build Artifacts:**

Each release produces:
- **9 binary archives** (or fewer if experimental targets fail)
- **18+ signature files** (.sig and .crt for each archive)
- **2 SBOM files** (JSON and XML formats)
- **1 security audit report** (JSON format)
- **1 SLSA provenance** (JSON attestation)

## Developer Workflow

### Normal Development Cycle

```bash
# 1. Create feature branch
git checkout -b feat/add-json-output

# 2. Make changes and commit with conventional format
git commit -m "feat(output): add JSON format support

Implements --json flag for machine-readable output.

$feat"

# 3. Push and create PR
git push origin feat/add-json-output
gh pr create --fill

# 4. After review, merge to master
# (Squash-and-merge or merge commit - both work)

# 5. Wait for release-plz to create release PR
# (Usually within 1 minute)

# 6. Review the release PR
# - Check version bump is correct
# - Verify changelog entries
# - Ensure CI passes

# 7. Merge the release PR
# (Tag is created automatically)

# 8. Wait for release workflow to complete
# (Typically 45-60 minutes)

# 9. Verify release on GitHub and crates.io
```

### Hotfix Workflow

For critical bug fixes that need immediate release:

```bash
# 1. Create hotfix branch from master
git checkout -b hotfix/critical-bug

# 2. Fix the bug
git commit -m "fix(parser): prevent null pointer dereference

Fixes critical crash when processing empty input.

$fix"

# 3. Create and merge PR quickly
git push origin hotfix/critical-bug
gh pr create --title "Hotfix: Critical parser crash" --body "Fixes #123"

# 4. After merge, release-plz creates release PR immediately
# This will bump patch version (e.g., 0.2.0 → 0.2.1)

# 5. Fast-track the release PR
# (Can merge as soon as CI passes)

# 6. Release is published automatically
```

## Manual Intervention Points

While the system is largely automated, there are several points where manual action is required or recommended:

### 1. Release PR Review (Required)

**When:** release-plz creates a release PR
**Action:** Review and merge the PR
**Checklist:**

- [ ] Version bump follows semantic versioning correctly
- [ ] Changelog accurately represents changes
- [ ] All CI checks pass (tests, coverage, clippy, format)
- [ ] Breaking changes are properly documented
- [ ] No sensitive information in commit messages

**What to Look For:**

- **Incorrect version bump:** If release-plz suggests wrong version (e.g., patch instead of minor), you can manually edit `Cargo.toml` in the PR
- **Missing changelog entries:** Ensure important changes aren't excluded by `$no-changelog`
- **Breaking changes:** Verify that breaking changes are marked with `!` or `BREAKING CHANGE:`

### 2. Failed Builds (Conditional)

**When:** Release workflow fails during building
**Action:** Investigate and potentially trigger manual publish

Experimental platforms (Windows ARM64, RISC-V) may fail without blocking the release. However, if primary platforms fail:

```bash
# Check workflow runs
gh run list --workflow=release.yml

# View logs for failed run
gh run view <run-id> --log-failed

# If needed, manually trigger publish after fixing
gh workflow run publish-crate.yml -f version=0.2.0
```

### 3. Publication Skipping (Optional)

**When:** You want to create a GitHub release without publishing to crates.io

```bash
# Trigger release workflow manually with skip_publish
gh workflow run release.yml \
  -f version=0.2.0 \
  -f skip_publish=true
```

Use cases:
- Pre-release testing
- Internal releases
- Waiting for crates.io maintenance window

### 4. Manual crates.io Publication (Recovery)

**When:** Automatic publication failed but GitHub release succeeded

```bash
# Use the dedicated publish workflow
gh workflow run publish-crate.yml \
  -f version=0.2.0 \
  -f skip_verification=false
```

This workflow will:
- Verify the GitHub release exists
- Check if already published to crates.io
- Validate version in Cargo.toml
- Publish the crate
- Verify publication succeeded

## Repository Configuration

### Required Secrets

The automation requires two repository secrets:

#### 1. NH_RELEASE_PLZ_TOKEN

**Type:** Fine-grained Personal Access Token
**Purpose:** Create release PRs that trigger CI workflows
**Why Needed:** `GITHUB_TOKEN` cannot trigger workflows; a PAT is required

**Setup:**

1. Go to https://github.com/settings/personal-access-tokens/new
2. Create a fine-grained PAT with these permissions:
   - **Repository access:** Only select `nutthead/ruloc`
   - **Permissions:**
     - Contents: Read and write
     - Pull requests: Read and write
     - Workflows: Read and write
3. Save the token
4. Add to repository secrets:
   - Go to https://github.com/nutthead/ruloc/settings/secrets/actions
   - Create secret named `NH_RELEASE_PLZ_TOKEN`
   - Paste the token value

**Validation:**

The `release-pr.yml` workflow validates this token and provides helpful error messages if missing.

#### 2. CARGO_REGISTRY_TOKEN

**Type:** crates.io API Token
**Purpose:** Publish crates to crates.io
**Why Needed:** Required for `cargo publish`

**Setup:**

1. Log in to https://crates.io
2. Go to https://crates.io/me
3. Under "API Tokens", create new token:
   - Name: `ruloc-release`
   - Crate scopes: Select `ruloc` (or use "All crates")
   - Endpoints: `publish-new` and `publish-update`
4. Copy the token (you won't see it again!)
5. Add to repository secrets:
   - Go to https://github.com/nutthead/ruloc/settings/secrets/actions
   - Create secret named `CARGO_REGISTRY_TOKEN`
   - Paste the token value

**Validation:**

The `release.yml` workflow now validates this token before attempting to publish.

### Required Repository Settings

**Actions Permissions:**

Settings → Actions → General → Workflow permissions:
- ✅ **"Read and write permissions"** must be enabled
- ✅ **"Allow GitHub Actions to create and approve pull requests"** should be enabled

**Branch Protection (Recommended):**

For `master` branch:
- ✅ Require status checks to pass (CI Success)
- ✅ Require branches to be up to date
- ❌ Do NOT restrict who can push tags (automated system needs access)

**Releases:**

Settings → General → Features:
- ✅ Issues should be enabled
- ✅ Discussions (optional, for release announcements)

## Troubleshooting

### Common Issues

#### Release PR Not Created

**Symptom:** After merging to master, no release PR appears

**Possible Causes:**

1. **No releasable changes**
   ```bash
   # Check if there are any feat/fix commits since last release
   git log v0.1.1..HEAD --oneline | grep -E "^(feat|fix)"
   ```

2. **Path filters didn't match**
   ```bash
   # Release PR only triggers for changes to:
   # - src/**
   # - Cargo.toml
   # - Cargo.lock
   # - .github/workflows/release-pr.yml
   ```

3. **Token missing or invalid**
   ```bash
   # Check workflow run for validation errors
   gh run list --workflow=release-pr.yml --limit 5
   gh run view <run-id> --log
   ```

4. **Skip CI hint present**
   ```bash
   # Check if commit message contains [skip ci]
   git log -1 --pretty=%B
   ```

**Resolution:**

- Ensure commits follow conventional commit format
- Verify `NH_RELEASE_PLZ_TOKEN` is configured and valid
- Check workflow runs for error messages
- Manually trigger if needed: `gh workflow run release-pr.yml`

#### Tag Not Created After Merging Release PR

**Symptom:** Release PR merged but no tag created

**Possible Causes:**

1. **Version not changed in Cargo.toml**
   ```bash
   # Check if version was actually updated
   git diff HEAD~1 HEAD -- Cargo.toml
   ```

2. **Workflow prevented by skip-ci**
   ```bash
   # Check merge commit message
   git log -1 --pretty=%B
   ```

3. **GitHub Actions bot created the commit**
   ```bash
   # Check commit author
   git log -1 --pretty=%an
   ```

**Resolution:**

- Verify Cargo.toml version was updated in the release PR
- Check that merge commit doesn't contain `[skip ci]`
- Review workflow runs: `gh run list --workflow=release-plz.yml`

#### Release Build Fails

**Symptom:** Tag created but release workflow fails

**Possible Causes:**

1. **Compilation error on specific platform**
   - Check build logs for the failing target
   - Experimental targets (RISC-V, Windows ARM64) may fail without blocking

2. **Dependency resolution failure**
   - Cargo.lock might be out of sync
   - Upstream dependency yanked from crates.io

3. **Test failure**
   - Tests run during `cargo package --locked`
   - Must pass for publication

**Resolution:**

```bash
# View failed build logs
gh run view <run-id> --log-failed

# Test locally with cross-compilation
cross build --target x86_64-unknown-linux-gnu --release

# If experimental target failed, this is expected
# Main platforms must succeed for release to complete
```

#### crates.io Publication Fails

**Symptom:** GitHub release created but crates.io publication fails

**Possible Causes:**

1. **Missing or invalid CARGO_REGISTRY_TOKEN**
   ```bash
   # Check if token validation step passed
   gh run view <run-id> --log | grep "Validate cargo registry token"
   ```

2. **Version already published**
   ```bash
   # Check if version exists on crates.io
   curl -s https://crates.io/api/v1/crates/ruloc/0.2.0 | jq .
   ```

3. **Rate limiting**
   - crates.io may rate limit publish requests
   - Wait and retry with publish-crate.yml workflow

**Resolution:**

```bash
# Verify token is configured
gh secret list | grep CARGO_REGISTRY_TOKEN

# Manually retry publication
gh workflow run publish-crate.yml -f version=0.2.0

# If version already exists, cannot republish
# Must create a new version (bump patch and create new tag)
```

### Debug Mode

To debug the release process without creating actual releases:

```bash
# Fork the repository
# Update .release-plz.toml to test in fork:
git_release_enable = false
publish = false

# This will create PRs and test version bumping without:
# - Creating git tags
# - Publishing GitHub releases
# - Publishing to crates.io

# Restore settings before production use
```

### Logging and Diagnostics

All workflows provide detailed logging:

```bash
# List recent workflow runs
gh run list --limit 10

# View specific run
gh run view <run-id>

# Download logs for offline analysis
gh run download <run-id> --name logs

# Watch live workflow
gh run watch
```

Key log sections to check:
- **Token validation:** Confirms secrets are configured
- **Version detection:** Shows calculated version bump
- **Changelog generation:** Preview of generated changelog
- **Build matrix:** Shows which platforms succeeded/failed
- **Signature verification:** Confirms Cosign signing worked

## Security and Attestation

### Artifact Signing

All release artifacts are signed using [Sigstore](https://www.sigstore.dev/)'s keyless signing:

**What is Signed:**
- Binary archives (.tar.gz, .zip)
- Each artifact gets two files:
  - `.sig` - Cosign signature
  - `.crt` - X.509 certificate containing identity

**How it Works:**
1. GitHub Actions obtains OIDC token from GitHub
2. Cosign uses token to request certificate from Fulcio (Sigstore CA)
3. Certificate embeds workflow identity (repository, ref, commit SHA)
4. Artifact is signed with ephemeral key
5. Signature and certificate uploaded to release

**Verification:**

Users can verify releases were built by the official workflow:

```bash
# Download artifact and its signature/certificate
VERSION="0.2.0"
PLATFORM="x86_64-unknown-linux-gnu"
curl -LO "https://github.com/nutthead/ruloc/releases/download/v${VERSION}/ruloc-${VERSION}-${PLATFORM}.tar.gz"
curl -LO "https://github.com/nutthead/ruloc/releases/download/v${VERSION}/ruloc-${VERSION}-${PLATFORM}.tar.gz.sig"
curl -LO "https://github.com/nutthead/ruloc/releases/download/v${VERSION}/ruloc-${VERSION}-${PLATFORM}.tar.gz.crt"

# Verify signature (requires cosign)
cosign verify-blob \
  --certificate "ruloc-${VERSION}-${PLATFORM}.tar.gz.crt" \
  --signature "ruloc-${VERSION}-${PLATFORM}.tar.gz.sig" \
  --certificate-identity-regexp "^https://github.com/nutthead/ruloc/\\.github/workflows/release\\.yml@refs/tags/v.*" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  "ruloc-${VERSION}-${PLATFORM}.tar.gz"
```

### SLSA Provenance

[SLSA](https://slsa.dev/) (Supply-chain Levels for Software Artifacts) Build Level 3 attestations are generated:

**What's Included:**
- Build environment details
- Source repository and commit SHA
- Build parameters and dependencies
- Workflow that performed the build

**Accessing Provenance:**

```bash
# Via GitHub's attestations API
gh attestation verify <artifact> --owner nutthead

# Provenance is also attached to the release
```

### Software Bill of Materials (SBOM)

CycloneDX SBOMs are generated in JSON and XML formats:

**Contents:**
- All direct and transitive dependencies
- Version constraints and resolved versions
- License information
- Component hashes

**Usage:**

```bash
# Download SBOM from release assets
curl -LO "https://github.com/nutthead/ruloc/releases/download/v${VERSION}/sbom.json"

# Analyze with SBOM tools
sbom-tool analyze sbom.json

# Check for known vulnerabilities
grype sbom:./sbom.json
```

## Advanced Topics

### Versioning Strategy

ruloc follows strict [Semantic Versioning 2.0.0](https://semver.org/):

- **Major (1.0.0):** Breaking API changes
- **Minor (0.1.0):** New features, backward compatible
- **Patch (0.0.1):** Bug fixes, backward compatible

**Pre-1.0.0 Behavior:**

Before 1.0.0, the project is considered experimental:
- Breaking changes bump minor version (0.1.0 → 0.2.0)
- New features bump minor version
- Bug fixes bump patch version

**Post-1.0.0 Behavior:**

After 1.0.0, full semver applies:
- Breaking changes bump major version (1.0.0 → 2.0.0)
- New features bump minor version (1.0.0 → 1.1.0)
- Bug fixes bump patch version (1.0.0 → 1.0.1)

**cargo-semver-checks Integration:**

release-plz uses `cargo-semver-checks` to detect breaking API changes automatically:

- Compares public API surface between versions
- Detects removed items, changed signatures, etc.
- Suggests major bump even if commits only indicate minor changes

### Changelog Customization

The changelog format can be customized in `.release-plz.toml`:

**Current Groups:**

- ⭐ Features - `feat:` commits or `$feat` tag
- 🐛 Bug Fixes - `fix:` commits or `$fix` tag
- 📚 Documentation - `docs:` commits or `$docs` tag
- 🧪 Testing - `test:` commits or `$test` tag
- 🔨 Refactor - `refactor:` commits or `$refactor` tag
- ⚡ Performance - `perf:` commits or `$perf` tag
- 🎨 Styling - `style:` commits or `$style` tag
- 📦 Build System - `build:` commits or `$build` tag
- 👷 CI/CD - `ci:` commits or `$ci` tag
- 🧹 Miscellaneous - `chore:` commits, `misc:` commits, `$chore` tag, or `$misc` tag
- ⏪ Reverts - `revert:` commits or `$revert` tag
- 🔐 Security - Commits containing "security"

**Adding Custom Groups:**

Edit `.release-plz.toml`:

```toml
commit_parsers = [
    # Add your custom parser
    { message = "^experiment", group = "🧬 Experiments" },

    # ... existing parsers ...
]
```

### Dependency Update Strategy

Current configuration: `dependencies_update = false`

This means:
- Dependencies are **NOT** automatically updated in release PRs
- Cargo.lock remains unchanged unless manually updated
- Aligns with `--locked` flags used throughout CI/release workflows

**Rationale:**

- **Reproducibility:** Locked dependencies ensure consistent builds
- **Explicit control:** Dependency updates reviewed in dedicated PRs
- **Security:** Updates can be tested separately before releases

**To Update Dependencies:**

```bash
# Create dedicated PR for dependency updates
git checkout -b chore/update-dependencies
cargo update
cargo test  # Ensure updates don't break anything
git commit -m "chore(deps): update dependencies

$(cargo tree --depth 1)

$chore"
```

### Platform Support Matrix

| Platform | Target Triple | Status | Cross-compile | Notes |
|----------|--------------|--------|---------------|-------|
| **Linux x64 glibc** | x86_64-unknown-linux-gnu | ✅ Stable | No | Primary platform |
| **Linux x64 musl** | x86_64-unknown-linux-musl | ✅ Stable | No | Static linking |
| **Linux ARM64 glibc** | aarch64-unknown-linux-gnu | ✅ Stable | Yes | Raspberry Pi 64-bit |
| **Linux ARM64 musl** | aarch64-unknown-linux-musl | ✅ Stable | Yes | Static ARM64 |
| **Linux ARMv7** | armv7-unknown-linux-gnueabihf | ✅ Stable | Yes | Raspberry Pi 32-bit |
| **macOS Apple Silicon** | aarch64-apple-darwin | ✅ Stable | No | M1/M2/M3/M4 |
| **Windows x64** | x86_64-pc-windows-msvc | ✅ Stable | No | Standard Windows |
| **Windows ARM64** | aarch64-pc-windows-msvc | ⚠️ Experimental | No | Surface ARM |
| **RISC-V 64** | riscv64gc-unknown-linux-gnu | ⚠️ Experimental | Yes | RISC-V hardware |

**Experimental Platform Behavior:**

- Marked with `experimental: true` in build matrix
- Uses `continue-on-error` to prevent blocking releases
- Binaries included only if build succeeds
- Failures logged but don't fail the workflow

### Release Cadence

There is no fixed release schedule. Releases are created on-demand when:

1. **Features accumulate:** Multiple new features ready for users
2. **Bug fixes:** Critical or high-priority bugs fixed
3. **Security updates:** Vulnerabilities patched
4. **Dependency updates:** Important upstream updates integrated

**Typical Cadence:**

- **Minor versions:** Every 2-4 weeks (with accumulated features)
- **Patch versions:** As needed for bug fixes (may be weekly)
- **Major versions:** When breaking changes are necessary (infrequent)

**Forcing a Release:**

To create a release without code changes:

```bash
# Manually bump version in Cargo.toml
sed -i 's/version = "0.1.1"/version = "0.1.2"/' Cargo.toml

# Update changelog manually if needed
# Then commit and push
git commit -am "chore(release): release version 0.1.2"
git push

# release-plz will detect version change and create release
```

### Emergency Rollback

If a release is severely broken:

1. **Yank from crates.io:**
   ```bash
   cargo yank --vers 0.2.0 ruloc
   ```

2. **Delete/deprecate GitHub release:**
   ```bash
   gh release delete v0.2.0 --yes
   # Or mark as pre-release to hide it
   gh release edit v0.2.0 --prerelease
   ```

3. **Create hotfix release:**
   ```bash
   # Fix the issue, create new release
   # Version MUST be higher (0.2.1, not 0.2.0)
   # Yanked versions cannot be republished
   ```

4. **Communicate:**
   - Update issue tracker
   - Post to discussions/announcements
   - Update README with known issues

**Note:** Yanking doesn't remove the version from crates.io, only prevents new projects from using it. Existing projects with Cargo.lock can still download it.

---

## Summary

The ruloc release process is designed to be:

- **Automated:** Minimal manual intervention required
- **Reliable:** Comprehensive testing and validation at each stage
- **Secure:** Cryptographic signing and provenance for all artifacts
- **Transparent:** Clear audit trail from commit to release
- **Developer-friendly:** Conventional commits make versioning intuitive

For questions or issues with the release process, consult this document or open an issue in the repository.

**Key Takeaways:**

1. Use conventional commits for all changes
2. Review release PRs carefully before merging
3. Monitor workflow runs for any failures
4. Verify releases on both GitHub and crates.io
5. Keep secrets (PAT and crates.io token) up to date

Happy releasing! 🚀
