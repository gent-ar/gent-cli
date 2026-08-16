# Standalone onboarding

The standalone first-run path is deliberately discovery-first and side-effect
free. It does not rely on the Flutter app and never installs, authenticates,
or starts a provider automatically.

## First run

Install (or explicitly update) Gent from a signed GitHub release. Choose a
published tag, verify its bootstrap asset, then execute it:

```sh
version=vX.Y.Z
curl -fLO "https://github.com/gent-ar/gent-cli/releases/download/$version/gent-install.sh"
curl -fLO "https://github.com/gent-ar/gent-cli/releases/download/$version/gent-install.sh.sigstore.json"
cosign verify-blob gent-install.sh --bundle gent-install.sh.sigstore.json \
  --certificate-identity-regexp "^https://github.com/gent-ar/gent-cli/.github/workflows/release.yml@refs/tags/$version$" \
  --certificate-oidc-issuer https://github.com/login/oauth
sh gent-install.sh --version "$version"
```

The script verifies the archive's GitHub OIDC Sigstore bundle and its manifest
before replacing either binary. It supports macOS arm64/x86_64 and Linux x86_64.
Use `GENT_VERSION=vX.Y.Z` to pin a release and `--force` only after reviewing a
new release. It keeps immutable version pairs and atomically switches both
launchers through one `current` pointer, so an interrupted update keeps the
previous pair runnable. This installer is deliberately user-invoked; `gentd`
does not self-update or expose a daemon update protocol. Ensure `~/.local/bin`
(or `GENT_INSTALL_DIR/bin`) is on `PATH`.

For an installed runtime, `gent update apply --version vX.Y.Z
--expected-sha256 <digest> --consent` is an equally explicit external handoff.
It downloads and verifies the release tag's installer bootstrap before invoking
it as a child process. The digest must be the target archive digest in its
signed manifest. The installer refuses activation if `gentd` holds the chosen
data directory's host lock, so stop that daemon first. It does not use a
release feed, a background timer, or an in-process binary replacement.

Windows x86_64 follows the same verified-bootstrap rule using
`gent-install.ps1` and `gent-install.ps1.sigstore.json` from that tag. Its
default runtime root is `%LOCALAPPDATA%\Gent`; add `%LOCALAPPDATA%\Gent\bin`
to `PATH`. The installer stages the immutable runtime pair and a signed native
launcher, publishes the launchers before atomically replacing a validated
`current.json` pointer, and never uses a `.cmd` argument-forwarding wrapper.
It does not use a symlink or replace a running binary.

1. Run `gent doctor`. It starts the local daemon if needed and reports Claude,
   Codex, Node.js, MCP observer state, private-bridge availability, executable
   identity, and remediation as JSON.
2. Review an explicit dependency action with `gent deps plan install claude`
   or `gent deps plan install codex`.
3. An install/update request must include `--consent`, for example
   `gent deps install claude --consent`. The daemon re-fetches the reviewed
   plan and active host epoch, then runs only its fixed shell-free vendor
   command. It records acceptance and the terminal result under one durable
   receipt. Use `--idempotency-key <key>` to retry the exact action; an
   ambiguous previously accepted effect is marked `unprovable`, never rerun.
4. Keep provider execution disabled until an unexpired signed compatibility
   entry and the required redacted live evidence exist. A discovered executable
   is not approval to launch it.

To assess an already-verified offline manifest, start `gentd` with
`--compatibility-cache <path>` and one or more
`--compatibility-key <key-id:lowercase-hex>` values (or their corresponding
`GENT_COMPATIBILITY_*` environment variables). This only revalidates local
signed data; it neither downloads a manifest nor starts a provider.

`gent doctor` is read-only discovery. It reports private Claurst integration as
`notConfigured` in this public repository; it never accepts Claurst endpoint,
credential, billing, or routing configuration.

## Current authority boundary

The shipped standalone daemon remains in observer mode. It has no provider
spawn, MCP process, Git mutation, automation engine, pairing transport, or
network-listener authority. The local IPC socket exists only for the versioned
`gent` ↔ `gentd` protocol.

Provider live-capture, private bridge, observer parity, and app cutover are
separate gates described in [implementation status](implementation-status.md).
They cannot be completed by an onboarding command or an unsigned local build.
