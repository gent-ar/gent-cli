# Sandboxed Autonomous Execution

## Status

Gent does not currently launch a provider. This document is therefore a
launch-authority contract, not a claim that the observer daemon contains a
provider today.

`Autonomous` and `Bypass` are durable permission modes. The one-time
`--consent-bypass` flag only changes into Bypass; it is never a required flag
for later `gent`, `gentd`, or app connections. A future provider launch in
either broad mode must fail before spawn unless its daemon-owned launcher has
verified OS sandbox enforcement. It must never silently retry unsandboxed.

## Permission model

| Mode | Prompt behavior | Sandbox requirement |
| --- | --- | --- |
| Default | Prompts unless an exact tool/category approval is durable | None |
| Plan | Read-only; non-read is denied | None |
| Auto-Accept Edits | Reads and edits are automatic | None |
| Autonomous | Reads, edits, and commands are automatic; other categories still prompt | Required |
| Bypass | No category-level permission prompts | Required |

The pure `gent-core` evaluator receives a non-serializable
`SandboxEnforcement` value from the future daemon process edge. If it is not
`Enforced`, Autonomous and Bypass return `SandboxRequired` before exact or
category approvals are considered. A client, protocol frame, SQLite record,
or provider output cannot assert that a sandbox exists.

## Required containment contract

The future launcher builds a canonical, immutable profile from the selected
workspace and policy, then rechecks the provider executable lock immediately
before launching that exact binary. The profile must minimally constrain:

- workspace read/write roots and no ambient home-directory access;
- network disabled by default, with separately reviewed egress policy when
  required;
- an allowlist of inherited environment variables, with credentials excluded;
- process-tree containment, resource ceilings, and no child escape path.

The launcher must bind the profile digest, backend identity, and enforcement
result to the durable run before the process is considered active. These facts
are daemon-owned diagnostics only; they are never sent by clients as authority
claims. A lock change, unsupported profile, failed backend preflight, or failed
attestation produces a terminal sandbox failure with zero provider spawn.

## Platform strategy

| Platform | Future supported path | Current broad-mode result |
| --- | --- | --- |
| Linux | Landlock filesystem policy plus a separately enforced network/process boundary | Denied until implemented and preflighted |
| macOS standalone CLI | A Gent-owned, Developer-ID-signed helper bundle with App Sandbox and hardened-runtime proof | Denied; `sandbox-exec` is not an acceptable security claim |
| Windows | AppContainer/restricted token plus a Job Object for the full process tree | Denied until implemented and preflighted |

The application may later provide a platform-specific signed helper, but that
does not relax the public process boundary. `gent-cli` remains responsible for
rejecting a requested required sandbox that cannot be enforced.

## Crate boundary

`gent-types` holds the serializable mode and runtime-only sandbox result value.
`gent-core` makes the pure no-prompt/fail-closed decision. `gent-ports` will
define the sandboxed process-launch port. `gent-drivers` implements platform
launchers at the operating-system edge. `gent-runtime` records trusted launch
facts and `gentd` is the only composition root. `gent-cli` and Flutter remain
protocol clients and may only read diagnostics.

## Required proof before authority

- A missing, unsupported, or failed backend causes zero launches.
- Paths outside the profile are inaccessible; child processes cannot escape.
- Ambient credentials are absent from the provider environment.
- Network policy is enforced rather than merely passed as a provider flag.
- A changed executable between lock resolution and sandbox spawn is rejected.
- Resume follows the same enforcement and lock recheck.
- macOS, Linux, and Windows claims each have platform-native integration
  evidence; an unsupported platform advertises no broad provider authority.

Provider authority remains blocked by the separate transcript, compatibility,
private Claurst, MCP, Git, and lifecycle gates in `implementation-status.md`.

## macOS delivery decision

The standalone terminal and the native app must use the same Gent-owned helper
protocol. A terminal process cannot claim App Sandbox containment merely from
an in-memory profile or a provider's own sandbox flag. Apple documents App
Sandbox inheritance only for an appropriately entitled child of a sandboxed
app; its recommended packaging route is a signed embedded helper or XPC
service. Gent therefore needs a separately signed helper bundle that proves
its code signature, App Sandbox entitlement, hardened runtime, and exact
parent/helper identity before it can return `Enforced`.

The current native-app Release entitlement file was inspected read-only on
2026-08-18 and explicitly has `com.apple.security.app-sandbox` set to false.
Gent must not treat that app process as a sandbox parent. This is not a request
to modify the app: the Gent release needs an independently updateable helper
delivery path so future provider and containment fixes ship with Gent alone.

The helper design must satisfy all of the following before macOS authority can
be enabled:

1. CI signs and notarizes the helper with a Gent-owned identity and verifies
   the exact entitlements and hardened-runtime flag after packaging.
2. The daemon verifies the helper's immutable lock and obtains a per-launch
   attestation bound to the provider lock/profile before it delegates spawn.
3. The helper gives provider children only the minimum inherited static rights;
   the daemon supplies normalized prompt/context input over private local IPC.
4. A macOS integration test proves denied filesystem/network/child-escape
   operations, bounded process-tree teardown, and zero spawn on a bad lock or
   invalid signature.

Until all four proofs exist, the macOS sandbox-preflight port remains
unavailable. This is deliberately stricter than invoking
`sandbox-exec` or trusting a provider CLI's advertised sandbox mode.
