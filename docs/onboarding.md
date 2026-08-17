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
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
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
data directory's host lock after a bounded drain wait. On updates where the
signed supervisor is required: it stages and probes the exact paired successor
over local IPC before the pointer switch and rolls back on successor failure.
An update handoff rejects a release without that signed supervisor. It does not
use an in-process binary replacement.

## Automatic updates

The signed updater companion registers a per-user scheduler during installation.
Use these commands to inspect, re-enable, or disable it:

```sh
gent update auto enable --interval-seconds 21600
gent update auto status
gent update auto disable
```

`gent update auto run` is also available for a one-shot check. GitHub `latest`
is untrusted discovery and can only choose a stable tag. The helper downloads
that tag's installer and Sigstore bundle, verifies its tag-bound GitHub OIDC
identity, then delegates to the same installer, staged health check, idle host
lock, and rollback path as an explicit update. It serializes runs, bounds each
operation, and records exponential-backoff state after failures. It never
replaces `gentd` in process or starts provider work. On Windows x86_64, the
same command invokes the installed signed `gent-auto-update.ps1` helper and
registers a uniquely named per-user Scheduled Task. That helper verifies the
selected tag's signed PowerShell bootstrap before it delegates to the same
idle-lock and staged local-IPC-health path; a failed candidate is never
selected, preserving the prior immutable pair.

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

For a runtime-release status check, the equivalent explicit profile is
`--runtime-update-check-authority --runtime-release-cache <path>
--runtime-release-trust <path>`. The trust document is the Sigstore-verified
public file published with runtime-release metadata; it can replace individual
`--runtime-release-key <key-id:lowercase-hex>` arguments. This profile only
revalidates cached signed metadata and cannot fetch, stage, or activate Gent.

For an approved offline planning audit, add
`--runtime-update-plan-authority --runtime-update-attempt-id <id>` with the
same cache and trust inputs. It records the signed release's eligibility in the
SQLite ledger and closes ingress when the pure lifecycle reducer requires it.
It intentionally stops before archive staging, health checks, or supervisor
handoff; those effects remain unavailable until a separately evidenced daemon
authority phase can acknowledge them after process exit.

The paired staged successor may subsequently use
`--runtime-update-recover-authority --runtime-update-attempt-id <id>` with the
same trusted cache inputs. That profile is for the external supervisor only:
it refuses any release, staging receipt, daemon version, or old host epoch
mismatch and leaves ingress closed on failure. It confirms the durable handoff,
binds its local endpoint while still closed, then fences/opens the successor
epoch; it is not a fetch, install, or provider-authority command.

While either explicit profile is running, `gent update status --attempt-id <id>`
can read that attempt's durable stage, revision, failure, host epoch, and
ingress state through the local protocol. It cannot fetch, schedule, stage, or
activate an update.

`gent doctor` is read-only discovery. It reports private Claurst integration as
`notConfigured` in this public repository; it never accepts Claurst endpoint,
credential, billing, or routing configuration.

## Current authority boundary

The shipped standalone daemon remains in observer mode. It has no live provider
spawn/lifecycle ingress, MCP process, Git mutation, automation engine, pairing
transport, or network-listener authority. The local IPC socket exists only for
the versioned `gent` ↔ `gentd` protocol.

Four Claude/Codex evidence cells remain capture-required: Claude
persistent-permission, compaction, malformed-tolerance; and Codex
malformed-tolerance. Claurst requires an authenticated app-private
bridge and private CI. A future Flutter launch must enforce one active
writer/host epoch and protocol compatibility; a single-user standalone install
does not require a legacy migration or deployed fence-aware app release. These
gates are described in [implementation status](implementation-status.md).
