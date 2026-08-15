# Standalone onboarding

The standalone first-run path is deliberately discovery-first and side-effect
free. It does not rely on the Flutter app and never installs, authenticates,
or starts a provider automatically.

## First run

1. Run `gent doctor`. It starts the local daemon if needed and reports Claude,
   Codex, Node.js, MCP observer state, private-bridge availability, executable
   identity, and remediation as JSON.
2. Review an explicit dependency action with `gent deps plan install claude`
   or `gent deps plan install codex`.
3. An install/update request must include `--consent`, for example
   `gent deps install claude --consent`. The current observer daemon returns
   `installerNotConfigured`; it does not silently substitute a package manager
   or run an installer.
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
