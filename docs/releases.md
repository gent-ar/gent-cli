# Release artifacts

Gent releases are tag-only GitHub releases. The repository never treats an
ordinary CI build, an unsigned archive, or a local developer binary as a
release artifact.

## What a release creates

Pushing a `v*` tag builds `gent` and `gentd` for Linux x86_64, macOS x86_64
and arm64, and Windows x86_64. Each target produces exactly these published
files:

- one archive containing both binaries;
- a SHA-256 sidecar;
- a JSON manifest naming the archive, target, version, digest, size, and
  contained binaries;
- a Sigstore bundle for each of the preceding files.

The package tool fixes archive metadata to `SOURCE_DATE_EPOCH`, derived from
the tagged commit. This makes archive construction deterministic for identical
binary inputs. It does not claim independently rebuilt Rust binaries are
bit-for-bit reproducible across hosts.

The workflow signs every published file keylessly with GitHub Actions OIDC.
It needs `id-token: write` only in the packaging jobs; no long-lived signing
key or credential is stored in the repository.

## Verify a downloaded archive

First check the archive digest and manifest without network access:

```sh
python3 tools/verify-release.py gent-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz \
  --manifest gent-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.manifest.json \
  --checksum gent-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sha256
```

Then verify the corresponding Sigstore bundle. The certificate issuer must be
GitHub and its identity must name this repository’s `release.yml` workflow and
a version tag:

```sh
cosign verify-blob gent-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz \
  --bundle gent-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sigstore.json \
  --certificate-identity-regexp '^https://github.com/gent-ar/gent-cli/.github/workflows/release.yml@refs/tags/v.+$' \
  --certificate-oidc-issuer https://github.com/login/oauth
```

Reject a release if either verification fails, the manifest’s target/version
does not match the intended download, or a required sidecar is missing. Signed
Gent release artifacts are separate from signed provider-compatibility entries
and never authorize a provider binary by themselves.
