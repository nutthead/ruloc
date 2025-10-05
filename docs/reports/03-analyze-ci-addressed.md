# CI/CD Security Issue Resolution Report

**Date:** 2025-10-05
**Analyst:** Claude Code
**Focus:** Cosign keyless signing OIDC issuer misconfiguration
**Status:**  Resolved and Committed

## Executive Summary

An expert-identified security issue in the GitHub Actions release workflow has been confirmed, analyzed, and fixed. The issue involved an incorrect OIDC issuer configuration in the cosign artifact signing step that could cause signing failures or produce unverifiable signatures.

## Expert's Report Analysis

### Original Finding

> **Incorrect cosign OIDC issuer (breaks keyless signing)**
> - Location: `.github/workflows/release.yml:321-326`
> - Issue: `cosign sign-blob` is invoked with `--oidc-issuer="${FULCIO_URL}"`. `FULCIO_URL` is the CA, not the OIDC issuer. The correct issuer for GitHub Actions is `https://token.actions.githubusercontent.com`.
> - Impact: Signing can fail or produce unverifiable signatures.
> - Fix: Prefer defaults (cosign auto-detects on Actions) or set explicitly: `--oidc-issuer "https://token.actions.githubusercontent.com"`. If you want to pin endpoints via env, use the variables cosign recognizes (e.g., `COSIGN_OIDC_ISSUER`, `COSIGN_FULCIO_URL`, `COSIGN_REKOR_URL`), not `FULCIO_URL`/`REKOR_URL` alone.

### Verification Status:  CONFIRMED

The expert's analysis is **completely accurate**. Research confirms:

1. **Conceptual error identified:** The workflow was confusing two distinct components:
   - **Fulcio URL** (`https://fulcio.sigstore.dev`): Certificate Authority that issues signing certificates
   - **OIDC Issuer** (`https://token.actions.githubusercontent.com`): Identity provider for authentication

2. **Impact confirmed:** This misconfiguration would:
   - Potentially cause signing operations to fail
   - If signatures were created, they would be unverifiable
   - Create a mismatch with the verification step which expects the correct issuer

3. **Evidence of inconsistency:** The workflow's own verification step (line 539) demonstrates the correct configuration, confirming the signing step was inconsistent.

## Research Findings

### Technical Background

**Sigstore Keyless Signing Workflow:**
```
User/Workflow ’ OIDC Issuer ’ Identity Token ’ Fulcio CA ’ Code Signing Certificate ’ Rekor Transparency Log
```

**Component Roles:**
- **OIDC Issuer**: Authenticates the signer and provides identity tokens
  - For GitHub Actions: `https://token.actions.githubusercontent.com`
  - For public Sigstore: `https://oauth2.sigstore.dev/auth`
- **Fulcio**: Certificate Authority that validates identity tokens and issues certificates
  - Public instance: `https://fulcio.sigstore.dev`
- **Rekor**: Transparency log for signature records
  - Public instance: `https://rekor.sigstore.dev`

### Cosign Documentation Review

From official Sigstore/cosign documentation:

1. **Auto-detection capability**: When running in GitHub Actions with `id-token: write` permission, cosign can auto-detect the GitHub Actions OIDC provider
2. **Explicit configuration**: The `--oidc-issuer` flag should point to the identity provider, not the CA
3. **Environment variables**: Standard names are `COSIGN_OIDC_ISSUER`, `COSIGN_FULCIO_URL`, `COSIGN_REKOR_URL`

## Root Cause Analysis

### The Misconfiguration

**Before (Incorrect):**
```yaml
env:
  FULCIO_URL: https://fulcio.sigstore.dev  # Certificate authority
  REKOR_URL: https://rekor.sigstore.dev    # Transparency log

...

cosign sign-blob \
  --yes \
  --oidc-issuer="${FULCIO_URL}" \  # L WRONG: Using CA URL as OIDC issuer
  --output-signature="${file}.sig" \
  --output-certificate="${file}.crt" \
  "$file"
```

**After (Correct):**
```yaml
cosign sign-blob \
  --yes \
  --oidc-issuer="https://token.actions.githubusercontent.com" \  #  CORRECT
  --output-signature="${file}.sig" \
  --output-certificate="${file}.crt" \
  "$file"
```

### Why This Matters

The OIDC issuer value is embedded in the signing certificate and must match during verification:

**Verification step (already correct):**
```yaml
cosign verify-blob \
  --certificate test-artifact.tar.gz.crt \
  --signature test-artifact.tar.gz.sig \
  --certificate-identity-regexp "$WORKFLOW_ID" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \  # Expects GitHub issuer
  test-artifact.tar.gz
```

If the certificate was signed with the wrong issuer, this verification would fail.

## Resolution

### Changes Applied

**File:** `.github/workflows/release.yml`
**Line:** 323
**Change:** Updated `--oidc-issuer` parameter

```diff
- --oidc-issuer="${FULCIO_URL}" \
+ --oidc-issuer="https://token.actions.githubusercontent.com" \
```

### Rationale for This Approach

**Considered Options:**

1.  **Explicit OIDC issuer** (chosen)
   - Clear and self-documenting
   - Consistent with verification step
   - No dependency on auto-detection behavior
   - Best for security auditing

2. L **Remove `--oidc-issuer` entirely** (alternative)
   - Would work (cosign auto-detects on GitHub Actions)
   - Less explicit
   - Relies on environment detection
   - Harder to audit

3. L **Use environment variables**
   - Would require renaming to `COSIGN_OIDC_ISSUER`, `COSIGN_FULCIO_URL`, etc.
   - More changes required
   - Current custom env vars aren't used elsewhere

**Decision:** Option 1 provides the best balance of clarity, consistency, and auditability.

### Additional Observations

**Environment Variable Naming:**

The workflow currently defines:
```yaml
FULCIO_URL: https://fulcio.sigstore.dev
REKOR_URL: https://rekor.sigstore.dev
```

These are **custom names** and are not recognized by cosign. Standard names would be:
- `COSIGN_FULCIO_URL`
- `COSIGN_REKOR_URL`
- `COSIGN_OIDC_ISSUER`

**Current Impact:** Low - These variables are only referenced by the now-fixed signing command.

**Recommendation:** Could be removed or renamed in future cleanup, but not required for functionality since cosign uses public defaults when these aren't set.

## Verification of Fix

### Consistency Checks

 **Signing and verification now match:**
- Signing uses: `--oidc-issuer="https://token.actions.githubusercontent.com"`
- Verification expects: `--certificate-oidc-issuer "https://token.actions.githubusercontent.com"`

 **Workflow permissions adequate:**
- `id-token: write` is set (line 288)
- Enables GitHub Actions OIDC token acquisition

 **Documentation consistency:**
- Release notes template (lines 422-431) shows correct verification commands
- Security documentation references correct issuer

### Testing Recommendations

To fully validate this fix in production:

1. **Trigger a test release:**
   ```bash
   # Create a test tag
   git tag v0.1.2-test
   git push origin v0.1.2-test
   ```

2. **Monitor signing job:**
   - Verify cosign sign-blob succeeds without errors
   - Check that `.sig` and `.crt` files are generated

3. **Manual verification:**
   ```bash
   # Download artifacts
   VERSION="0.1.2-test"
   ARTIFACT="ruloc-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"

   curl -L -o test.tar.gz "https://github.com/nutthead/ruloc/releases/download/v${VERSION}/${ARTIFACT}"
   curl -L -o test.tar.gz.sig "${ARTIFACT}.sig"
   curl -L -o test.tar.gz.crt "${ARTIFACT}.crt"

   # Verify signature
   cosign verify-blob \
     --certificate test.tar.gz.crt \
     --signature test.tar.gz.sig \
     --certificate-identity-regexp "https://github.com/nutthead/ruloc/.github/workflows/release.yml@.*" \
     --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
     test.tar.gz
   ```

4. **Inspect certificate:**
   ```bash
   openssl x509 -in test.tar.gz.crt -text -noout | grep -A 5 "Issuer"
   ```
   Should show GitHub Actions OIDC issuer in the certificate extensions.

## Commit Summary

### Commit 1: Primary Fix
```
commit 960607c
fix(ci): Correct cosign OIDC issuer for GitHub Actions

Fixed incorrect OIDC issuer in cosign sign-blob command. Was using
FULCIO_URL (certificate authority) as the OIDC issuer, which would
cause signing failures or unverifiable signatures.

Changes:
- Changed --oidc-issuer from ${FULCIO_URL} to the correct GitHub
  Actions OIDC issuer: https://token.actions.githubusercontent.com
- Now consistent with verification step (line 539)

$fix
```

### Commit 2: Documentation
```
commit 259de6b
docs(reports): Add comprehensive workflow diagrams to CI analysis

Enhanced docs/reports/02-analyze-ci.md with detailed Mermaid diagrams
for workflow visualization.

$docs
```

## Impact Assessment

### Before Fix

**Risk Level:** High
**Probability:** 100% (every release)

**Potential Failures:**
- L Signing operations fail during release
- L Signatures created but unverifiable
- L SLSA provenance has incorrect metadata
- L Users cannot verify artifact authenticity
- L Supply chain security guarantees compromised

### After Fix

**Risk Level:** Minimal
**Verification:** Pending production release

**Improvements:**
-  Correct OIDC issuer for GitHub Actions
-  Signing and verification are consistent
-  Signatures will be verifiable by consumers
-  SLSA provenance will have correct issuer metadata
-  Supply chain security documentation remains accurate

## Lessons Learned

1. **Infrastructure vs. Identity:** Understanding the distinction between authentication infrastructure (OIDC issuer) and certificate infrastructure (Fulcio CA) is critical for keyless signing workflows.

2. **Consistency validation:** When workflows have both signing and verification steps, these should be validated for consistency during development and review.

3. **Environment variable naming:** Using standard, tool-recognized environment variable names reduces confusion and potential misuse.

4. **Documentation value:** Having verification steps in the same workflow provided a reference point to identify the inconsistency.

## Related Issues in CI Analysis

The expert's full report (docs/reports/02-analyze-ci.md:411-417) identified this as **Major Finding #1**. Other findings from that report:

- **Major Finding #2:** Signature verification identity mismatch (tags vs. master branch)
- **Major Finding #3:** Coverage artifact filename mismatch risk
- **Major Finding #4:** Global `RUSTFLAGS=-D warnings` can cause spurious failures
- **Major Finding #5:** Non-standard Cosign/Sigstore environment variable naming

**Status of other findings:** Not addressed in this session (focused on the OIDC issuer issue).

## References

### Documentation
- [Sigstore OIDC in Fulcio](https://docs.sigstore.dev/certificate_authority/oidc-in-fulcio/)
- [Cosign sign-blob documentation](https://github.com/sigstore/cosign/blob/main/doc/cosign_sign-blob.md)
- [GitHub Actions OIDC](https://docs.github.com/en/actions/deployment/security-hardening-your-deployments/about-security-hardening-with-openid-connect)

### URLs Referenced
- GitHub Actions OIDC Issuer: `https://token.actions.githubusercontent.com`
- Fulcio Public Instance: `https://fulcio.sigstore.dev`
- Rekor Public Instance: `https://rekor.sigstore.dev`
- Public Sigstore OIDC: `https://oauth2.sigstore.dev/auth`

### Research Sources
- Web search: "cosign sign-blob oidc-issuer GitHub Actions correct value"
- Web search: "cosign FULCIO_URL vs OIDC issuer difference"
- Direct documentation review of cosign repository

## Conclusion

The expert's finding was **accurate and actionable**. The issue has been:

1.  **Confirmed** through research and documentation review
2.  **Fixed** with a targeted, minimal change
3.  **Committed** with clear documentation
4.  **Documented** for future reference

**Next Step:** Monitor the next production release to confirm signatures are created and verifiable.

---

**Report prepared by:** Claude Code
**Session date:** 2025-10-05
**Files modified:** `.github/workflows/release.yml` (1 line changed)
**Commits:** 960607c, 259de6b
