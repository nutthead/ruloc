# How These Workflows Operate (Triggers, Actors, Dependencies)

This section explains, with detailed sequence and flow diagrams, how each GitHub Actions workflow is triggered, who/what can trigger it, and how they depend on one another.

Key actors
- Developer: pushes commits, opens PRs, merges PRs.
- Fork Contributor: opens PRs from a fork.
- Maintainer: manually dispatches workflows, manages releases.
- Merge Queue: GitHub merge queue invoking `merge_group` events.
- Scheduler: Weekly cron triggering CI on Mondays 00:00 UTC.
- Release-plz Action: automation that opens release PRs or tags versions.
- External services: Codecov, Sigstore (Fulcio/Rekor), crates.io.

Event-to-workflow mapping (with path filters)
- ci.yml
  - pull_request: opened/synchronize/reopened when files under `src/**`, `Cargo.{toml,lock}`, `.github/workflows/**`, `.tarpaulin.toml` change.
  - push: branch `master` with same path filters.
  - merge_group: `checks_requested`.
  - workflow_dispatch: manual run by a maintainer.
  - schedule: weekly cron.
- release-pr.yml
  - push: branch `master` with same Rust-related path filters; skips when commit message contains `[skip ci]`.
- release-plz.yml
  - push: branch `master` with same filters; guarded to avoid loops (not `github-actions[bot]`, no `[skip ci]`); runs only if `Cargo.toml` version changed vs previous commit.
- release.yml
  - push: tags matching `v*.*.*`.
  - workflow_dispatch: manual, with inputs `version` and `skip_publish`.
- publish-crate.yml
  - workflow_dispatch: manual, with inputs `version` and `skip_verification`.

Note on concurrency
- ci.yml uses a concurrency group keyed by workflow name and PR number or ref; new runs cancel in-progress ones for the same PR/branch.

Overall interdependency (high level)

```mermaid
flowchart LR
  %% Actors
  Dev[Developer]
  Maint[Maintainer]
  Sch[Scheduler]
  MQ[Merge Queue]

  %% Events
  EPR[pull_request]
  EPUSH[push master via merge]
  EMG[merge_group]
  ECRON[schedule]
  EDISP_CI[workflow dispatch ci]
  EDISP_REL[workflow dispatch release]
  EDISP_PUB[workflow dispatch publish]
  ETAG[push tag vX.Y.Z]

  %% Workflows
  CI[ci.yml]
  RPR[release-pr.yml]
  RPLZ[release-plz.yml]
  REL[release.yml]
  PUB[publish-crate.yml]

  %% CI triggers
  Dev --> EPR --> CI
  Sch --> ECRON --> CI
  MQ --> EMG --> CI
  Maint --> EDISP_CI --> CI

  %% Merge to master produces push event
  CI --> Merge[Merge PR to master]
  Maint --> Merge
  Merge --> EPUSH

  %% Release automation on push to master
  EPUSH --> RPR
  EPUSH --> RPLZ
  RPLZ --> CreateTag[Create tag vX.Y.Z] --> ETAG
  ETAG --> REL

  %% Manual releases and publish
  Maint --> EDISP_REL --> REL
  REL --> GHRel[GitHub Release]
  REL -->|optional| Crates
  Maint --> EDISP_PUB --> PUB
  PUB --> Crates
```

## Global Interactions (Sequence)

```mermaid
sequenceDiagram
  autonumber
  actor Dev as Developer
  actor Fork as Fork Contributor
  actor Maint as Maintainer
  actor MQ as Merge Queue
  actor Sch as Scheduler
  participant GH as GitHub Events
  participant CI as ci.yml
  participant RP as release-pr.yml
  participant RTag as release-plz.yml
  participant Rel as release.yml
  participant Pub as publish-crate.yml
  participant CC as Codecov
  participant Sig as Sigstore
  participant Cr as crates.io

  Dev->>GH: Open PR
  Fork->>GH: Open PR from fork
  MQ->>GH: checks_requested (merge_group)
  Sch->>GH: Weekly cron (Mon 00:00 UTC)
  Maint->>GH: workflow_dispatch

  GH->>CI: Trigger (PR/push/merge_group/dispatch/schedule)
  Note over CI: Concurrency cancels in-progress runs for same ref

  par Quick checks
    CI->>CI: fmt, clippy, doc
  and Advisory scan
    CI->>CI: audit + deny (soft fail)
  end

  alt Quick checks succeed
    CI->>CI: unit-tests (matrix)
    CI->>CI: coverage (tarpaulin)
    CI->>CC: Upload coverage
    opt Same-repo PR and report exists
      CI->>GH: Comment coverage summary
    end
  else Quick checks fail
    CI->>GH: Report failure
  end

  opt merge_group
    CI->>GH: Set CI Status check
  end

  Maint->>GH: Merge PR to master with version bump
  GH->>RTag: Trigger release-plz (version changed)
  RTag->>GH: Create tag vX.Y.Z
  GH->>Rel: Trigger release on tag
  Maint->>Rel: Or manual dispatch
  Rel->>Sig: Sign and attest
  Rel->>GH: Create GitHub Release
  alt skip_publish = false
    Rel->>Cr: cargo publish
  end
  Rel->>GH: Verify signatures and indexing
  Maint->>Pub: Manual crates.io publish fallback
  Pub->>Cr: Publish if needed
```

## ci.yml (Detailed Flowchart)

```mermaid
flowchart TD
  %% Actors and events
  Dev[Developer] --> EPR[pull_request]
  Maint[Maintainer] --> Merge[Merge PR to master]
  Merge --> EPUSH[push master via merge]
  MQ[Merge Queue] --> EMG[merge_group]
  Sch[Scheduler] --> ECRON[schedule]
  Maint --> EDISP[workflow dispatch]

  %% Event routing: path filters apply only to PR and push
  EPR --> PF[Check path filters]
  EPUSH --> PF
  EMG --> CONC
  ECRON --> CONC
  EDISP --> CONC

  PF -->|match| CONC[Apply concurrency: cancel in progress]
  PF -->|no match| EXIT((Exit))

  %% Jobs
  CONC --> QC[quick-check: fmt, clippy, doc]
  CONC --> SEC[security: audit and deny, soft fail]
  QC --> QRES{quick-check ok?}
  QRES -->|no| GATE[ci-success gate]
  QRES -->|yes| UT[unit-tests: OS+target matrix]
  UT --> COV[coverage: tarpaulin]
  COV --> COMM{same-repo PR and report?}
  COMM -->|yes| COMMENT[Comment coverage on PR]
  COMM -->|no| SKIP[Skip comment]
  COV --> CODECOV[Upload to Codecov]
  COMMENT --> GATE
  SKIP --> GATE
  CODECOV --> GATE
  SEC --> GATE
  GATE --> MERGEQ{merge_group event?}
  MERGEQ -->|yes| SETCHECK[Set CI Status check]
  MERGEQ -->|no| DONE((Done))
```

## release-pr.yml (Release PR automation)

```mermaid
sequenceDiagram
  autonumber
  actor Maint as Maintainer
  participant GH as GitHub
  participant W as release-pr.yml
  participant RP as release plz action
  participant PR as Release PR
  participant CI as ci.yml

  Maint->>GH: Merge PR to master (push event)
  GH->>W: Trigger release-pr.yml (paths filtered)

  alt Commit message has skip-ci hint
    W->>GH: Skip workflow (job condition)
  else Proceed
    W->>W: Validate NH_RELEASE_PLZ_TOKEN exists
    alt Token missing
      W-->>Maint: Fail with guidance to create PAT
    else Token present
      W->>W: Checkout with PAT
      W->>W: Setup Rust and cache
      W->>RP: Run release-plz command release-pr
      RP->>PR: Open or update Release PR using PAT
      PR->>GH: PR opened or synchronize
      GH->>CI: Trigger ci.yml for Release PR
    end
  end
```

```mermaid
flowchart TD
  A[Merge PR to master] --> B[Check paths]
  B -->|no match| X((Exit))
  B -->|match| C{Commit message contains skip-ci hint?}
  C -->|yes| X
  C -->|no| D{Secret NH_RELEASE_PLZ_TOKEN set?}
  D -->|no| E[[Fail with guidance]]
  D -->|yes| F[Checkout with PAT]
  F --> G[Install toolchain and cache]
  G --> H[release-plz release-pr]
  H --> I[Release PR opened/updated]
```

## release-plz.yml (Tagging on version bump)

```mermaid
sequenceDiagram
  autonumber
  actor Dev as Developer
  participant GH as GitHub
  participant W as Release-plz Tag Workflow
  participant RP as release-plz-action (release)
  participant Tag as vX.Y.Z tag
  participant Rel as release.yml

  Dev->>GH: Merge PR to master with Cargo.toml change
  GH->>W: Trigger release-plz.yml (guarded against loops)
  W->>W: Diff Cargo.toml: PREV_VERSION vs CURR_VERSION
  alt Version changed
    W->>RP: release-plz command 'release'
    RP->>Tag: Create annotated tag vCURR
    Tag->>Rel: Trigger release.yml
  else No change
    W->>GH: Exit (no tag)
  end
```

```mermaid
flowchart TD
  A[Merge PR to master] --> B[Check paths]
  B -->|no match| X((Exit))
  B -->|match| C{Pusher is GitHub Actions bot?}
  C -->|yes| X
  C -->|no| D{Commit contains skip-ci hint?}
  D -->|yes| X
  D -->|no| E{Was Cargo.toml modified?}
  E -->|no| X
  E -->|yes| F["Read PREV_VERSION from HEAD~1"]
  F --> G[Read CURR_VERSION from HEAD]
  G --> H{PREV_VERSION != CURR_VERSION?}
  H -->|no| X
  H -->|yes| I[release-plz release]
  I --> J[Create tag vCURR]
  J --> K[Triggers release.yml]
```

## release.yml (Full release pipeline)

```mermaid
sequenceDiagram
  autonumber
  actor Maint as Maintainer
  participant GH as GitHub
  participant R as Release Workflow
  participant Prep as prepare-release
  participant Sec as security-scan
  participant Build as build-binaries (matrix)
  participant Att as attestation/sign
  participant Cliff as generate-changelog
  participant PubRel as publish-release
  participant PubCr as publish-crate (optional)
  participant Ver as verify-release
  participant Sig as Sigstore
  participant GHRel as GitHub Release
  participant Cr as crates.io

  GH-->>R: Trigger on tag push
  Maint->>R: Manual dispatch with inputs
  R->>Prep: Extract version, validate, compare to Cargo.toml
  Prep-->>R: Version output
  par Parallel stage
    R->>Sec: Run audit and deny checks
    R->>Sec: Generate CycloneDX SBOM
    Sec-->>R: Upload security artifacts
    R->>Build: Build binaries for all targets
    R->>Build: Package and compute checksums
    Build-->>R: Upload build artifacts
    R->>Cliff: Generate changelog
    Cliff-->>R: Upload changelog
  end
  R->>Att: Download artifacts
  R->>Att: Generate build provenance
  R->>Att: Cosign sign blobs
  Att->>Sig: Write to Sigstore services
  R->>PubRel: Create GitHub Release
  PubRel->>GHRel: Publish release assets
  alt Publish to crates
    R->>PubCr: Publish crate to crates.io
    PubCr->>Cr: Crate published
  else Skip publish
    R->>Ver: Continue without crates.io publish
  end
  R->>Ver: Verify signatures and crate availability
```

```mermaid
flowchart TD
  A[Trigger: tag push or manual] --> B[prepare-release: determine version]
  B -->|invalid/mismatch| X[[Fail]]
  B -->|ok| C{Run in parallel}
  C --> D[security-scan: audit/deny/SBOM]
  C --> E[build-binaries: matrix build + package + checksum]
  C --> F[generate-changelog]
  D --> G[attestation/sign]
  E --> G
  F --> H[publish-release: create GitHub release]
  G --> H
  H --> I{skip_publish?}
  I -->|yes| J[verify-release]
  I -->|no| K[publish-crate: cargo publish]
  K --> J
  J --> Done((Done))
```

## publish-crate.yml (Manual crates.io fallback)

```mermaid
sequenceDiagram
  autonumber
  actor Maint as Maintainer
  participant GH as GitHub
  participant P as Publish Crate Workflow
  participant GHRel as GitHub Release
  participant Cr as crates.io

  Maint->>GH: workflow_dispatch(version, skip_verification)
  GH-->>P: Trigger publish-crate.yml
  P->>P: Normalize/validate version (strip leading 'v')
  P->>GHRel: Check release tag vVERSION exists
  P->>Cr: Query if version already published
  alt Already published
    P-->>Maint: Abort with guidance (yank option)
  else Not published
    P->>P: Checkout tag, build, cargo package --list
    P->>Cr: cargo publish
    opt skip_verification = false
      P->>Cr: Poll until indexed
    end
  end
```

---

# CI/CD Workflows Analysis (as of 2025-10-05)

This report reviews all workflows in `.github/workflows` and maps their dependencies, highlights issues, and proposes concrete improvements and potential rewrites. File/line references use the repository’s current state.

## Workflow Inventory

- `ci.yml`: Continuous Integration for PRs, pushes to `master`, merge queue, weekly schedule. Stages: quick checks → unit tests (matrix) → coverage; security audit runs in parallel; a final “CI Success” gate consolidates results.
- `release-pr.yml`: Creates/updates a release PR on pushes to `master` using `release-plz` and a PAT (`NH_RELEASE_PLZ_TOKEN`).
- `release-plz.yml`: On version bump in `Cargo.toml` on `master`, runs `release-plz` in “release” mode to tag a version (which triggers the release pipeline).
- `release.yml`: Full release pipeline triggered by `v*.*.*` tags or manual dispatch. Stages: prepare version → security scan/SBOM → build (matrix) → attestation/signing → changelog → GitHub Release → crates.io publish → verification.
- `publish-crate.yml`: Manual fallback to publish an existing tagged release to crates.io.

## Dependency Map

High-level flow (→ indicates “triggers/depends on”):

```
Push/PR → ci.yml (quick-check → unit-tests → coverage) & security (parallel) → ci-success

master push with version bump → release-plz.yml (release) → tag vX.Y.Z → release.yml

release.yml: prepare-release → [security-scan, build-binaries → attestation, generate-changelog]
            → publish-release → [publish-crate (optional), verify-release]

publish-crate.yml is manual only; independent of release.yml except for tags existing.
release-pr.yml is orthogonal; manages release PRs on master.
```

Explicit job dependencies inside `ci.yml` and `release.yml` are sound and avoid flakiness from implicit ordering.

## Major Findings

1) Incorrect cosign OIDC issuer (breaks keyless signing)
- Location: `.github/workflows/release.yml:321-326`.
- Issue: `cosign sign-blob` is invoked with `--oidc-issuer="${FULCIO_URL}"`. `FULCIO_URL` is the CA, not the OIDC issuer. The correct issuer for GitHub Actions is `https://token.actions.githubusercontent.com`.
- Impact: Signing can fail or produce unverifiable signatures.
- Fix:
  - Prefer defaults (cosign auto-detects on Actions) or set explicitly: `--oidc-issuer "https://token.actions.githubusercontent.com"`.
  - If you want to pin endpoints via env, use the variables cosign recognizes (e.g., `COSIGN_OIDC_ISSUER`, `COSIGN_FULCIO_URL`, `COSIGN_REKOR_URL`), not `FULCIO_URL`/`REKOR_URL` alone.

2) Signature verification ties identity to master branch, but releases are tagged
- Location: `.github/workflows/release.yml:534-540`.
- Issue: Verification uses `--certificate-identity-regexp` with `.../release.yml@refs/heads/master`. Release runs are triggered by tags (`refs/tags/v*`).
- Impact: Verification will fail even if signatures are correct.
- Fix options:
  - Match tags: `.../.github/workflows/release.yml@refs/tags/v.*`.
  - Or use a more robust regex anchored to workflow path only (and optionally repo owner): `.../.github/workflows/release.yml@.*` combined with `--certificate-oidc-issuer https://token.actions.githubusercontent.com`.

3) Coverage artifact filename mismatch risk
- Location: `ci.yml` Codecov upload and comment steps expect `target/tarpaulin/cobertura.xml` (`ci.yml:234-261, 243-249, 261`).
- In `.tarpaulin.toml` the XML output is enabled, but tarpaulin often writes `tarpaulin-report.xml` by default for XML. If the filename is `tarpaulin-report.xml`, the Codecov upload and PR comment steps won’t find it (guarded by `hashFiles` and `fail_ci_if_error: false`).
- Fix options:
  - Standardize the filename: change CI to `files: target/tarpaulin/tarpaulin-report.xml` and read the same in the comment step; or
  - Configure tarpaulin to emit `cobertura.xml` explicitly (if supported) and keep CI as-is; or
  - Switch to `cargo-llvm-cov` which produces consistent artifacts across platforms.

4) Global `RUSTFLAGS=-D warnings` can cause spurious failures
- Location: `ci.yml:44-49`.
- Issue: Setting `-D warnings` globally affects dependencies during `cargo test` builds, not just your crate(s).
- Impact: Third-party warnings can fail CI unexpectedly.
- Fix: Remove the global `RUSTFLAGS` and rely on `cargo clippy -- -D warnings` (`ci.yml:84-85`) and `RUSTDOCFLAGS` just for docs (`ci.yml:87-90`). If you want to gate compile-time warnings for the workspace only, use `RUSTFLAGS` with `--config warnings=...` per package or `workspace.lints` in `Cargo.toml`.

5) Cosign/Sigstore env naming
- Location: `release.yml:34-44` and `attestation` job.
- Issue: Using `FULCIO_URL`/`REKOR_URL` env keys is non-standard; cosign recognizes `COSIGN_FULCIO_URL`/`COSIGN_REKOR_URL`.
- Impact: URLs may be ignored if cosign arguments don’t override them.
- Fix: Rename envs to `COSIGN_FULCIO_URL` and `COSIGN_REKOR_URL`, or pass flags explicitly.

## Minor Findings

- Permissions scope in CI is broader than necessary
  - `ci.yml:51-55` grants `pull-requests: write` and `checks: write` globally. Only the coverage comment and merge-queue check creation actually need elevated perms. Narrow permissions at the job level where needed.

- Test parallelism is disabled
  - `ci.yml:189-190` forces `--test-threads=1`. Unless tests are flaky or depend on global state, allow default parallelism for speed and use per-test synchronization where needed.

- musl test target without dependency checks
  - `ci.yml:183-188` installs `musl-tools`, but many crates need additional system libs or `pkg-config`. Consider gating musl in a separate job or using `cross` for musl tests too for consistency.

- Release matrix breadth
  - `release.yml:154-190` builds a wide target set including `riscv64` and Windows ARM64. If these are experimental, mark them optional (non-blocking) or build them behind a flag to reduce release risk.

- Tool bootstrap consistency
  - You mix `cargo-binstall`, `taiki-e/install-action`, and `cargo install`. This is fine, but you can standardize on one approach and cache binaries (`~/.cargo/bin`) where appropriate.

## Opportunities and Improvements

Security and Supply Chain
- Enforce least privilege per job:
  - Keep `permissions` minimal at workflow level and elevate only in steps that need it (PR comments, checks API, release creation).
- Pin or verify third-party downloads:
  - You already pin actions by SHA (excellent). Consider pinning `curl` downloads in verification steps by checksum when feasible.
- Adopt provenance and signing best practices consistently:
  - Keep `actions/attest-build-provenance@v3` (good). Fix issuer flags (Major Finding #1/#2) to ensure verification passes.

Coverage
- Prefer `cargo-llvm-cov` for cross-platform speed and consistency. It integrates well with Codecov, supports branch coverage, and runs on macOS/Windows (tarpaulin is Linux-only). Example swap in CI:
  - Install: `taiki-e/install-action` with `tool: cargo-llvm-cov`.
  - Run: `cargo llvm-cov --workspace --lcov --output-path lcov.info`.
  - Upload: Codecov with `files: lcov.info`.
- If staying on tarpaulin, explicitly set the XML filename and keep CI paths aligned.

Build Speed
- Cache improvements:
  - You already use `Swatinem/rust-cache` (good). Optionally add `sccache` for larger codebases and enable it via env in build/test/coverage jobs.
- Matrix right-sizing:
  - Consider running full OS matrix on PRs only for changed paths (use `paths:` filters at job-level) and run the full matrix on `push` to `master` and `schedule`.

Release Process
- Unify `release-plz` flows:
  - Keep `release-pr.yml` for PRs and `release-plz.yml` for tags, or consolidate into a single workflow with two jobs keyed off inputs/conditions. Today’s split works but adds duplication.
- Guard crates.io publish behind an environment approval:
  - Use GitHub Environments with required reviewers for the `publish-crate` step to add a human-in-the-loop gate.
- Artifact naming and verification:
  - Ensure consistent naming across platforms; keep a single script that computes the expected asset name and checks presence before release creation.

DX/Observability
- Use `actions/setup-node` when invoking `npm` in CI coverage for clearer logs and optional caching.
- Promote release notes generation to a dedicated script committed to the repo for easier iteration and testing.

## Suggested Targeted Fixes (quick wins)

- Fix cosign issuer and envs:
  - Change `.github/workflows/release.yml:321-326` to either drop `--oidc-issuer` or set it to `https://token.actions.githubusercontent.com`.
  - Rename envs to `COSIGN_FULCIO_URL`/`COSIGN_REKOR_URL` or pass flags explicitly.
- Fix verification identity for tags:
  - Update `.github/workflows/release.yml:534` to `...release.yml@refs/tags/v.*` or a path-only regex.
- Align coverage file path:
  - Ensure tarpaulin writes the expected filename, then update `ci.yml:234-239, 243-249, 261` accordingly.
- Reduce global permissions in CI:
  - Move `pull-requests: write` and `checks: write` from `ci.yml:51-55` to only the jobs/steps that require them.
- Remove global `RUSTFLAGS=-D warnings` (`ci.yml:44-49`) and rely on clippy/doc flags.

## Possible Rewrites (optional, larger changes)

- Reusable workflow for Rust jobs
  - Create `.github/workflows/_rust-reusable.yml` with inputs (toolchain, targets, run-tests, coverage-tool). Have `ci.yml` and `release.yml` call it via `workflow_call` for consistency and less duplication.

- Switch coverage to `cargo-llvm-cov`
  - Replace `coverage` job with a reusable coverage workflow (Linux/macOS/Windows). Merge PR coverage summary via a prebuilt action or a small Node script committed in `scripts/`.

- Single `release-plz` workflow
  - Combine `release-pr` and `release-plz` into one file with two jobs keyed by event conditions, sharing setup/cache steps.

- Harden release verification
  - Add a job that downloads a random sample of assets and verifies both checksum and cosign signature using the corrected issuer/identity.

## Notes on Current Strengths

- Actions pinned by commit SHA throughout — excellent supply-chain hygiene.
- Thoughtful concurrency and merge-queue handling (`ci.yml:39-41`, `ci.yml:363-379`).
- Good caching via `Swatinem/rust-cache` and selective `save-if` on `master`.
- Clear PR-friendly behavior (skip comments for forks; `continue-on-error` for advisories).

## Next Steps Checklist

- [ ] Patch cosign issuer flag and env names in `release.yml`.
- [ ] Update verification identity to match tag refs.
- [ ] Decide on coverage tool (tarpaulin vs `cargo-llvm-cov`) and align paths.
- [ ] Scope CI permissions to only the steps that need them.
- [ ] Remove global `RUSTFLAGS=-D warnings`.
- [ ] Consider consolidating `release-plz` workflows and/or extracting a reusable Rust workflow.

If you want, I can open a PR applying the “quick wins” and scaffolding a reusable coverage workflow.
