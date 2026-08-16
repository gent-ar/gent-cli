# Release artifacts

Gent releases are tag-only GitHub releases. The repository never treats an
ordinary CI build, an unsigned archive, or a local developer binary as a
release artifact.

## What a release creates

Pushing a `v*` tag builds `gent` and `gentd` for Linux x86_64, macOS x86_64
and arm64, and Windows x86_64. Each target produces exactly these published
files:

- one archive containing the immutable `gent`/`gentd` pair. Windows also
  contains the signed `gent-launcher.exe`, copied to the stable PATH entries
  so no command-shell wrapper re-parses user arguments;
- a SHA-256 sidecar;
- a JSON manifest naming the archive, target, version, digest, size, and
  contained binaries;
- a Sigstore bundle for each of the preceding files.
- a target-specific `*.runtime-release.json` envelope signed with the release
  Ed25519 key, its Sigstore bundle, and a Sigstore-signed public trust file;
- signed `gent-install.sh`, `gent-install.ps1`, and
  `gent-activate-install.py`, and `gent-supervise-runtime-activation.py`
  bootstrap assets, each with a Sigstore bundle.

The package tool fixes archive metadata to `SOURCE_DATE_EPOCH`, derived from
the tagged commit. This makes archive construction deterministic for identical
binary inputs. It does not claim independently rebuilt Rust binaries are
bit-for-bit reproducible across hosts.

The workflow signs every published file keylessly with GitHub Actions OIDC.
It also uses the repository’s `GENT_RUNTIME_RELEASE_PRIVATE_KEY` secret only
to sign the compact runtime-update envelope. Its matching key id and public
key live in protected repository variables and are published in the signed
trust file; neither private key material nor credentials are stored in the
repository. High-assurance
deployments should download the release bootstrap and verify its bundle before
execution rather than using a moving source URL.

`GENT_RUNTIME_RELEASE_PRIVATE_KEY` is an Ed25519 PKCS#8 PEM. The metadata
signer uses only the Python standard library so the same signed envelope can
be produced and verified by the repository's macOS, Linux, and Windows gates.

The runtime trust document is not itself permission to replace a running
daemon. It only supplies public verification keys to the explicit cached
status profile; staging, health confirmation, supervised activation, and
rollback remain separate daemon authority gates.

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
