# Keyless Signing with Sigstore: A beginner's guide

## What is Software Signing and Why Does It Matter?

Imagine you order a package online. When it arrives, how do you know it's really from the store and not a fake package someone swapped out? You might check for:
- An official seal
- A tracking number
- The store's logo

Software signing works the same way! When you download a program, you want to know:
1. **Who made it** - Is this really from the official developer?
2. **It hasn't been tampered with** - Did anyone modify it after the developer released it?

Signing software is like putting a special seal on it that proves both of these things.

## The Old Way: GPG Signing (Like Having a Physical Key)

### How It Used to Work

Traditionally, developers used something called GPG (GNU Privacy Guard) to sign their software. Think of it like this:

1. **The Developer Creates a Special Key**
   - They make two keys: a private key (kept secret) and a public key (shared with everyone)
   - The private key is like a stamp only you own
   - The public key is like a sample of your stamp that anyone can check against

2. **Signing the Software**
   - Developer uses their private key to "stamp" the software
   - This creates a signature (like a wax seal on a letter)

3. **Verifying the Software**
   - Users download both the software and the signature
   - They use the developer's public key to check if the signature is valid
   - If it matches, they know it's authentic!

### The Problems with GPG

While GPG works, it has some annoying problems:

- **Key Management is Hard** - You have to store private keys somewhere safe (like in GitHub secrets). If someone steals your key, they can pretend to be you!
- **Keys Can Leak** - Accidentally committing a private key to git is a common mistake
- **Keys Expire** - You have to manage expiration dates and rotate keys
- **Key Distribution** - Users need to find and trust your public key
- **Revocation is Messy** - If your key is compromised, telling everyone not to trust it anymore is complicated

It's like having to carry around a physical key everywhere and never losing it... forever!

## The New Way: Keyless Signing with Sigstore

Sigstore invented a brilliant solution: **What if we didn't need to manage keys at all?**

### How Keyless Signing Works (The Magic!)

Instead of managing keys yourself, Sigstore does something clever:

1. **Use Your Identity** - You prove who you are using something you already have (like your GitHub login)
2. **Get a Temporary Certificate** - Sigstore gives you a short-lived certificate (expires in minutes)
3. **Sign Your Software** - Use that certificate to sign
4. **Record Everything** - The signature gets recorded in a public, tamper-proof log

It's like instead of owning a stamp, you walk into a notary office, prove your identity with your driver's license, they stamp your document for you, and record it in a permanent ledger!

### The Three Key Components

Let's understand what those URLs in our workflow mean:

```yaml
env:
  COSIGN_EXPERIMENTAL: 1
  FULCIO_URL: https://fulcio.sigstore.dev
  REKOR_URL: https://rekor.sigstore.dev
```

#### 1. **Fulcio** (The Certificate Authority)

**What it does:** Issues short-lived signing certificates

**The ELI5 version:**
- Fulcio is like a notary public or a DMV
- You prove your identity (via GitHub, Google, etc.)
- Fulcio checks "Yep, this is really you!" and gives you a certificate
- The certificate expires in 10-20 minutes (so even if someone steals it, it's useless shortly after)

**In our workflow:**
- GitHub Actions proves its identity using an OIDC token (a special code that proves the workflow is running legitimately)
- Fulcio issues a certificate tied to that specific GitHub Actions run
- The certificate says "This signature came from workflow X in repository Y"

#### 2. **Rekor** (The Transparency Log)

**What it does:** Records every signature in a public, tamper-proof ledger

**The ELI5 version:**
- Rekor is like a blockchain or a permanent public ledger
- Every time someone signs something, it gets recorded here
- You can't delete or modify entries (they're cryptographically linked)
- Anyone can look up when something was signed and by whom

**Why this matters:**
- Even though the signing certificate expires in minutes, the Rekor entry lasts forever
- If something weird happens (like someone signing malicious code), there's a permanent record
- You can audit the entire history of signatures

**In our workflow:**
- After we sign a release artifact, the signature gets uploaded to Rekor
- Rekor returns a timestamp and proof that it was recorded
- Users can verify the signature by checking Rekor (even months later when the certificate has expired)

#### 3. **Cosign** (The Signing Tool)

**What it does:** The command-line tool that coordinates everything

**The ELI5 version:**
- Cosign is like the app on your phone that talks to the notary (Fulcio) and the ledger (Rekor)
- It handles all the complicated cryptography
- It makes signing and verifying as easy as running one command

**The `COSIGN_EXPERIMENTAL=1` flag:**
- This enables "keyless mode"
- Without this flag, Cosign expects you to provide your own keys (the old GPG way)
- With this flag, Cosign uses Fulcio and Rekor instead

## How Our Release Workflow Uses It

Here's the step-by-step flow when our release workflow runs:

### 1. **GitHub Actions Starts Running**
```yaml
permissions:
  id-token: write  # This allows GitHub to create identity tokens
```
- GitHub creates a special OIDC token that proves "This is workflow release.yml running in the ruloc repository"

### 2. **Cosign Requests a Certificate**
```yaml
- name: Sign artifacts with cosign
  run: |
    cosign sign-blob \
      --yes \
      --oidc-issuer="${FULCIO_URL}" \
      ...
```
- Cosign sends the OIDC token to Fulcio
- Fulcio verifies the token and issues a short-lived certificate
- The certificate says: "This signature was created by GitHub Actions workflow at https://github.com/your-org/ruloc/.github/workflows/release.yml"

### 3. **Sign the Artifacts**
- Cosign uses the certificate to sign each release file (ruloc-v1.0.0-linux.tar.gz, etc.)
- Creates `.sig` (signature) and `.crt` (certificate) files

### 4. **Upload to Rekor**
- Cosign automatically uploads the signature to Rekor
- Rekor records: "At 2025-10-05 17:30:42 UTC, workflow X signed artifact Y"
- Returns a timestamp and proof of inclusion

### 5. **Publish Everything**
- The release includes: the binary, the signature, and the certificate
- Users can verify these files

## How Users Verify Signatures

When someone downloads `ruloc-v1.0.0-linux.tar.gz`, they can verify it's authentic:

```bash
# Download the artifact, signature, and certificate
curl -L -O https://github.com/your-org/ruloc/releases/download/v1.0.0/ruloc-v1.0.0-linux.tar.gz
curl -L -O https://github.com/your-org/ruloc/releases/download/v1.0.0/ruloc-v1.0.0-linux.tar.gz.sig
curl -L -O https://github.com/your-org/ruloc/releases/download/v1.0.0/ruloc-v1.0.0-linux.tar.gz.crt

# Verify the signature
cosign verify-blob \
  --certificate ruloc-v1.0.0-linux.tar.gz.crt \
  --signature ruloc-v1.0.0-linux.tar.gz.sig \
  --certificate-identity-regexp "https://github.com/your-org/ruloc/.github/workflows/release.yml" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  ruloc-v1.0.0-linux.tar.gz
```

**What happens during verification:**
1. Cosign checks the certificate was issued by Fulcio
2. Checks the certificate identity matches our workflow
3. Checks the signature matches the file
4. Looks up the signature in Rekor to verify it was recorded
5. If everything matches: ✅ "Valid signature"

## The Benefits of Keyless Signing

### For Developers:
- ✅ **No secrets to manage** - No GPG_PRIVATE_KEY or GPG_PASSPHRASE in GitHub secrets
- ✅ **Nothing to leak** - No private keys to accidentally commit
- ✅ **No key rotation** - No expiration dates to worry about
- ✅ **Automatic setup** - Works out of the box with GitHub Actions OIDC

### For Users:
- ✅ **Strong provenance** - Know exactly which workflow created the release
- ✅ **Transparency** - Public audit log of all signatures
- ✅ **Tamper-proof** - Can't modify or delete signatures after the fact
- ✅ **Easy verification** - One command to verify everything

### For Everyone:
- ✅ **Better security** - Short-lived certificates mean smaller attack window
- ✅ **SLSA Level 3** - Meets high security standards for supply chain security
- ✅ **Free and open** - No cost, no lock-in

## Understanding the Environment Variables

Let's break down each one:

### `COSIGN_EXPERIMENTAL=1`
- **What it does:** Enables keyless signing mode in Cosign
- **Why we need it:** Without this, Cosign expects you to provide your own keys
- **The "experimental" name:** It's historical - keyless signing is actually production-ready now, but the flag name stuck

### `FULCIO_URL=https://fulcio.sigstore.dev`
- **What it does:** Tells Cosign where to get signing certificates
- **Why we need it:** Cosign needs to know which certificate authority to use
- **Alternative:** You could run your own Fulcio instance for private/air-gapped environments

### `REKOR_URL=https://rekor.sigstore.dev`
- **What it does:** Tells Cosign where to record signatures
- **Why we need it:** So signatures get logged in the public transparency log
- **Alternative:** You could run your own Rekor instance

## Real-World Analogy: The Complete Story

Imagine you're a famous author (the developer) releasing a new book (software):

### The Old Way (GPG):
1. You create a special signature stamp (private key)
2. You hire a security guard to protect your stamp 24/7
3. You stamp every book with it
4. Readers have a sample of your signature to compare against
5. **Problem:** If someone steals your stamp, they can forge your books forever!

### The New Way (Sigstore):
1. When you want to release a book, you go to a notary office (Fulcio)
2. You show your ID (GitHub OIDC token)
3. The notary verifies your identity and signs the book for you
4. They record it in a permanent public ledger (Rekor)
5. The notary's authorization expires in 10 minutes, but the ledger entry lasts forever
6. **Benefit:** No stamp to steal! Each book is notarized individually with a fresh authorization

## SLSA Level 3 Attestations

The workflow also generates SLSA (Supply-chain Levels for Software Artifacts) attestations:

```yaml
- name: Generate build provenance
  uses: actions/attest-build-provenance@v3.0.0
```

**What this adds:**
- A detailed record of exactly how the software was built
- Which workflow, commit, and build environment was used
- Cryptographically links the source code to the binary

Think of it as a "birth certificate" for your software that proves it was built legitimately by GitHub Actions from your repository's code.

## Common Questions

### Q: Why is it called "keyless" if certificates are technically keys?
**A:** The "keyless" refers to *you* not managing long-lived keys. The system still uses cryptographic keys internally, but they're temporary and handled automatically.

### Q: What if Fulcio or Rekor goes down?
**A:**
- Existing signatures remain valid (they're already in Rekor)
- You can't create *new* signatures until the service recovers
- Sigstore runs redundant infrastructure to minimize downtime
- Organizations can run their own instances for critical needs

### Q: Can I use this for private/internal software?
**A:** Yes! You can:
- Use the public Sigstore infrastructure (signatures are public but artifact content isn't shared)
- Run your own Fulcio + Rekor instances for complete privacy

### Q: What's the catch?
**A:** The main "catch" is:
- Requires internet access to Fulcio and Rekor during signing
- Requires trusting Sigstore's infrastructure (though it's open source and auditable)
- Verification requires checking Rekor (but this is automatic with Cosign)

## Summary

Keyless signing with Sigstore is like having a magical notary public that:
- Never loses your signature stamp (because you don't have one)
- Records every document in a tamper-proof ledger
- Allows anyone to verify authenticity years later
- Costs nothing and requires no key management

By setting these three environment variables, our release workflow gets enterprise-grade security with zero secret management:

```yaml
env:
  COSIGN_EXPERIMENTAL: 1          # Enable keyless mode
  FULCIO_URL: https://fulcio.sigstore.dev    # Certificate authority
  REKOR_URL: https://rekor.sigstore.dev      # Transparency log
```

It's the future of software signing, and it's available today! 🚀

## Additional Resources

- [Sigstore Official Documentation](https://docs.sigstore.dev/)
- [Cosign GitHub Repository](https://github.com/sigstore/cosign)
- [SLSA Framework](https://slsa.dev/)
- [How Sigstore Works (Video)](https://www.youtube.com/watch?v=nCUfwXVN2JM)
