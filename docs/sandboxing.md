# Sandboxed Autonomous Execution

## Status

A Windows PSEC backend and isolated native fixtures exist, but no helper has complete reviewed policy proof. Fixture results do not authorize a release.

Containment is chosen per conversation: `gent_types::Containment` on the conversation config record (`containment`: `unsandboxed`, the default, or `sandboxed`), projected on `AgentChatConversationSummary.containment` and set with `gent chat create --sandboxed` or an `agent-chat-conversation-config-v1` save. Changing it during an active turn returns `selectionSwitchBlockedByActiveTurn`.
The launcher choice is one rule, `Containment::requires_sandbox`: Unsandboxed launches Claude, Codex and Claurst through the ordinary host path in every mode, so local servers, previews and `git clone` work; Sandboxed launches through the contained path in every mode.
A failed sandbox admission fails the turn with a notice whose `failureCode` is `sandboxUnavailable` (no helper, pins or package on this device) or `sandboxAdmissionFailed`; it never retries unsandboxed. `--consent-bypass` still guards only the change into Bypass.

## Permission model

| Mode | Prompt behavior | Unsandboxed | Sandboxed |
| --- | --- | --- | --- |
| Default | Prompts unless an exact tool/category approval is durable | Host | Contained |
| Plan | Read-only; non-read is denied | Host | Contained |
| Auto-Accept Edits | Reads and edits are automatic | Host | Contained |
| Autonomous | Reads, edits, commands, and network tools are automatic | Host | Contained, evidence required |
| Bypass | No category-level permission prompts | Host | Contained, evidence required |

The pure `gent-core` evaluator receives a non-serializable `SandboxEnforcement` value from the daemon process edge. For a Sandboxed conversation that is not `Enforced`, Autonomous and Bypass return `SandboxRequired` before exact or category approvals are considered.
No client, protocol frame, SQLite record or provider output can assert that a sandbox exists; a workspace policy read without its conversation resolves as Sandboxed.

## Required containment contract

The sandbox launcher builds a canonical, immutable profile from the selected
workspace and policy, then rechecks the provider executable lock immediately
before launching that exact binary. The profile must minimally constrain:

- workspace read/write roots and no ambient home-directory access;
- outbound Internet access in Autonomous and Bypass without per-request
  approvals, enforced by the sandbox backend's declared network policy;
- inbound and host-local access constrained independently from outbound
  Internet access;
- an allowlist of inherited environment variables, with credentials excluded;
- process-tree containment, resource ceilings, and no child escape path.

### Windows two-environment boundary

One PSEC environment cannot hold both authorities. Native probes on Windows 11
build 26200 (2026-11-02) established why:

- `internetClient` with egress default allow and no proxy or ingress reached
  public TCP, DNS and HTTP. Unrelated host loopback, the host LAN address and
  the router all returned `WSAEACCES`, as did a connection to the environment's
  own loopback listener. A host connection to a container listener timed out.
- Proxy, package peer and `internetClient` together made
  `CreateProcessSecurityEnvironment` return `E_INVALIDARG`. A peer without the
  proxy mapping plus direct egress could not reach the gateway.
- `ingress.host_loopback=allow` opens every host listener. PSEC egress deny
  rules do not filter loopback, so it cannot be narrowed.
- A Job can hold processes from one PSEC environment only. Assigning an
  environment B process to a Job holding environment A returns
  `ERROR_ACCESS_DENIED`, in either order.
- `fs_enumerate` on a directory overrides `fs_read_only` on that same directory.
  A file grant inside a directory grant makes the directory enumerate-only, so
  the native policy drops file grants already covered by a directory grant.

So Windows runs two environments for one run:
| | Provider (A) | Commands (B) |
| --- | --- | --- |
| Network | PSEC proxy + package peer + `privateNetworkClientServer` | `internetClient`, egress default allow, no proxy, peer or ingress |
| Read | Profile read grants | Profile read grants, private state root, `%SystemRoot%`, `%ProgramFiles%`, `%ProgramFiles(x86)%` |
| Write | Profile write grants, private state root | Profile workspace write grants, private `tmp` and `home` |
| Job | Run Job with the full process count and half the memory and CPU | Sibling root Job with the same limits, one nested Job per command |

The launcher root holds one image named `bash.exe`, `pwsh.exe` and `cmd.exe`. The
gateway-mode environment selects them through `CLAUDE_CODE_GIT_BASH_PATH`,
`GENT_GIT_BASH_PATH`, `ComSpec`, `GENT_POWERSHELL_PATH` and the first PATH entry.
Each sends one request over a pipe in A's AppContainer namespace to the broker,
which checks the client's AppContainer SID and run Job, accepts only a fixed target enum
resolved to the pinned images, duplicates the shim's standard handles, accepts
only file and pipe handles, and refuses a working directory outside B's roots.
It proves B's exact capabilities, integrity and distinct SID on the suspended
process before resume. Console standard handles are relayed through pipes.
Failure exits 126 with `GENT-CONTAINED-SHELL-ERROR`; nothing runs in A instead.

B's environment drops `ANTHROPIC_`, `OPENAI_`, `GENT_`, `CUSTOM_OPENAI_`,
`CLAURST_` and `LLAMA_CPP_` prefixes, the proxy variables, names ending `_TOKEN`, `_API_KEY` or `_SECRET`, and any entry holding
the run bearer or gateway origin. PATH holds the pinned MSYS closure (`crates/gentd/windows-command-tools.json`), with the private `bash.exe` also as `sh.exe` and `npm`/`npx` launchers for the managed `bin`/`lib` layout, host Git and Node, and managed Node; `url.gent::http(s)://` routes `git` through `git-remote-gent`, which resolves the host before libcurl starts.
`claude`, `codex` and `home\.claurst` stay read-only for B.
After a normal exit, background processes stay in the nested Job until they exit
or the run ends. Killing a shim terminates its Job.

Autonomous and Bypass require `SandboxCommandEgress::BrokeredInternet` on the
shell runtime. It is bound into the profile digest, checked in
`permission_backend_registry` and `WindowsSandboxLauncher::launch_sandboxed`, and
provider identity flows through one exhaustive
`AgentChatProvider::contained_contract` mapping.

`platform/windows/provider-helper/two-env-fixture.ps1` exercises this on the real
helper. Known limits: B has no loopback, so local servers fail, `git` redirects to another host fail, and non-bare `git clone` fails with `Unable to create '.../refs/remotes/origin/HEAD.lock': No such file or directory` on unreadable workspace ancestors because enumerate grants are recursive (use `init` + `fetch` + `checkout`), `NODE_OPTIONS` carries `--preserve-symlinks --preserve-symlinks-main` because realpath stats the denied drive root, so Node resolves modules by their symlink path; stock MSYS tools die in PSEC unless staged beside the private
runtime, built from the private tree recorded in the runtime review; A resolves DNS and
can pass data to B through the workspace; Claude WebFetch fails in A because the pinned package proxy refuses CONNECT and its channels are half-duplex, so fetch from a command in B. This is not release acceptance.

The launcher must bind the profile digest, backend identity, and enforcement
result to the durable run before the process is considered active. These facts
are daemon-owned diagnostics only; they are never sent by clients as authority
claims. A lock change, unsupported profile, failed backend preflight, or failed
attestation produces a terminal sandbox failure. Native checks that require
the newly created token run while the provider is suspended; rejection kills
that process before resume. Unsupported-host checks must reject before
creating a provider, with no unsandboxed retry.

## Platform strategy

| Execution host / client | Current implementation | Current broad-mode result |
| --- | --- | --- |
| Windows host | `WindowsSandboxLauncher` implements the native launch port; PSEC/AppContainer, Jobs and external packaged proxy have scoped native evidence | Production denied: complete-helper allowlist, final package/pins and full provider/policy acceptance remain open |
| macOS host, CLI or app | Swift helper and Rust client implement bounded `prepare` denial only; there is no contained provider launch implementation | Denied before ordinary provider fallback; signed/notarized packaging alone is not containment |
| Linux host | Ordinary provider launcher and process-group lifetime control exist; no Linux implementation of `SandboxedProviderLaunch` is composed | Denied; a version-negotiated Landlock/network/process backend and native acceptance are still needed |
| Android local app | `LocalMobileCapabilities` has no local agent or PTY implementation | No local provider authority; Android app UID isolation is not a per-run Gent provider sandbox |
| Android or other paired client | Remote capability and permission requests target the execution owner | Follows the owner's backend and policy; a client OS or mode selection cannot supply containment proof |

Source audit: 2026-10-30 UTC. `permission_backend_registry::native`,
`SandboxProviderLauncher`, `RoutedClaurstStandaloneLauncher` and
`ManagedPermissionFactories` reject required containment on non-Windows
hosts. The macOS helper returns `denied` even after successful bookmark
authorization. Linux process groups and the app's Android UID boundary must
not be presented as an implemented provider sandbox. This audit did not run
native macOS/Linux tests or physical Android acceptance.

Ask and Auto-accept edits remain ordinary host modes without a Gent OS
containment requirement. Autonomous automatically permits reads, edits,
commands and network tools; other categories can still prompt. Bypass permits all
categories only after the same required containment gate. Plan/read-only is
a separate per-chat restriction, not an OS-sandbox attestation. Windows
component/transport passes do not deliver broad-mode availability on any
other platform, nor do they open the Windows production gate.

Platform primitives are available independently of this implementation:
[Apple App Sandbox inheritance](https://developer.apple.com/library/archive/documentation/Miscellaneous/Reference/EntitlementKeyReference/Chapters/EnablingAppSandbox.html),
[Linux Landlock](https://docs.kernel.org/userspace-api/landlock.html) and
[Android Application Sandbox](https://source.android.com/docs/security/app-sandbox).
Each still needs a Gent launch, resource, credential, lifetime and evidence
composition satisfying the contract above.

The application may later provide a platform-specific signed helper, but that
does not relax the public process boundary. `gent-cli` remains responsible for
rejecting a requested required sandbox that cannot be enforced.

## Crate boundary

`gent-types` holds the serializable mode and runtime-only sandbox result value.
`gent-core` makes the pure no-prompt/fail-closed decision. `gent-drivers` owns
the sandboxed process-launch port and implements platform
launchers at the operating-system edge. `gent-runtime` records trusted launch
facts and `gentd` is the only composition root. `gent-cli` and Flutter remain
protocol clients and may only read diagnostics.

## Required proof before authority

- A missing or unsupported backend causes zero provider launches; a failed
  post-creation proof prevents provider resume and tears down the owned tree.
- Paths outside the profile are inaccessible; child processes cannot escape.
- Ambient credentials are absent from the provider environment.
- Network policy is enforced rather than merely passed as a provider flag.
- A changed executable between lock resolution and sandbox spawn is rejected.
- Resume follows the same enforcement and lock recheck.
- macOS, Linux, and Windows claims each have platform-native integration
  evidence; an unsupported platform advertises no broad provider authority.

Provider authority remains blocked by the separate transcript, compatibility,
private Claurst, MCP, Git, and lifecycle gates in `implementation-status.md`.

## Windows implementation and release audit (2026-10-26)

Native source is in `platform/windows/provider-helper/`. The helper loads the
System32 PSEC API dynamically and rejects missing exports, unsupported version
or capability flags, invalid grants and failed launch proofs. It never retries
with a weaker token, Job-only containment or an ordinary provider launch.
PSEC creation uses suspended processes, explicit filesystem grants and drive
denials, separate inherited stdin/stdout/stderr pipes, inspected tokens and
descendant resource limits. The bounded handshake precedes provider resume
and binds the profile, run and native process creation identity. The process
owner retains the helper image lock and containment lifetime.

The last fully exercised native helper before the shell addition has SHA256
`d1b0b30b7b7d2eaf56079339cb5f5cfa4ba9fb6b53b1c89c4d48a6077d87007f`.
Its immutable validation artifact is outside this checkout under
`runtime-parity/windows-sandbox-candidate/d1b0b30b7b7d2eaf/` in the parent
workspace. Its proof manifest records source hashes and fixture locations.
Positive and negative fixtures cover workspace access, permissive outside
ACL denial, read-only authority, reparse/hardlink escapes, independent exact
stdio, inherited handles, descendants, memory/process ceilings and owner-death
cleanup. The peer gateway permits its approved stream while denying alternate
loopback ports, IPv6, direct public traffic and actual private-LAN egress with
an ordinary host positive control. Existing user ACLs were unchanged.

These results do not prove LAN ingress isolation. The provider can bind a
listener; a nonpeer host-loopback connection is denied. A cross-device LAN
negative lacks a reachable ordinary-listener positive control under the
ambient firewall. The narrowly scoped fixture-rule creation was denied, so
no rule was created. The peer proxy uses a bidirectional private-network
capability; firewall timeouts cannot establish a native ingress guarantee.
`COMPLETE_POLICY_HELPERS` in `crates/gentd/src/windows_sandbox_runtime.rs` is empty.
Its `require_complete_policy_proof` gate denies broad production authority.

Full Claude stream-json startup also requires a shell. The new source accepts
`--shell-exe` and `--shell-sha256`. The typed shell lock participates in the
profile digest, requires an explicit read-only runtime-parent grant and is
rechecked before launch. Native code verifies the image hash and retains a
read handle denying replacement/write/delete through the provider lifetime.
It rejects writable shell coverage and reparse ancestors, sets the exact
`CLAUDE_CODE_GIT_BASH_PATH`, and builds PATH from that parent plus Windows
System32/Windows. Runtime dependencies need separate explicit grants;
the controlled PATH does not grant filesystem access. A file-only grant was
insufficient for a contained child launch; a runtime-directory grant passed
the isolated child fixture. No ambient PATH, host home or provider credentials
are imported.

The shell fixture passed its four cases, but the resulting candidate SHA256
`bc7afa83dfc8c0747f92a68e640e6f1d8a35f20ea23374d92088ae0b4dfd82d9`
is unaccepted. Defender classified the new helper and copied probe as
`Trojan:Win32/Bearfoos.A!ml` before gateway regression. This detection is
unclassified by this project; it is not evidence of a false positive. No
quarantine restore, exclusion, policy change, rebuild or repack to bypass it
was performed. The earlier d1b0 artifact lacks the shell addition. Neither
artifact is a completed broad-mode distribution.

The release packaging work still required is concrete:

1. Review and resolve the detected artifact through the release security
   process. Produce an accepted, reviewed and signed distribution with a
   recorded signer, source revision, toolchain, artifact digest and signature
   verification. Signing alone proves neither containment nor AV acceptance.
2. Add the missing release staging step. CMake currently builds
   `gent-windows-sandbox.exe`; the daemon accepts only
   `<daemon-directory>/runtime/windows/provider-helper.exe`. No runtime path
   override is consulted. Staging/renaming must occur before the final package
   manifest and verification; a signed image's final digest is the pin.
3. Compile the daemon with that immutable lowercase SHA256 in
   `GENT_WINDOWS_SANDBOX_HELPER_SHA256`. Missing/malformed pins, missing helper,
   escaped canonical install paths and digest mismatches reject. Runtime file
   capture/recheck and the held no-replacement image handle remain required;
   the build variable is not a per-run environment override.
4. Record provider/shell locks and the finite reviewed runtime dependency
   file/directory manifest. The current real-provider smoke takes an explicit
   system-DLL file list and Git runtime root; its dependencies and installed
   versions are not yet a portable release manifest. Do not substitute a broad
   System32 grant or skip alias/read-only admission failures.
5. Rerun native positive/negative, exact-stdio, shell, resource/lifetime,
   cross-device network and authenticated contained Read/Write tests on the
   accepted final signed artifact and packaged daemon. Recheck unsupported
   hosts and absent/changed helper rejection. Only complete reviewed policy
   evidence for that exact digest can justify a complete-proof allowlist entry.

The repository has fixture scripts and CMake targets, but no finished Windows
release staging/signing/pinning pipeline or end-to-end reproduction document
for the accepted shell/provider package. The current manual build/probe logs
and ignored integration test are evidence for their exact artifacts, not a
reproducible accepted release procedure. The outstanding procedure must name
OS capability prerequisites, exact toolchain and dependency manifests, package
layout, signed hashes, bounded fixture commands, expected results and cleanup.

## macOS delivery decision

The standalone terminal and the native app must use the same Gent-owned helper
protocol. A terminal process cannot claim App Sandbox containment merely from
an in-memory profile or a provider's own sandbox flag. Apple documents App
Sandbox inheritance only for an appropriately entitled child of a sandboxed
app; its recommended packaging route is a signed embedded helper or XPC
service. Gent therefore needs a separately signed helper bundle that proves
its code signature, App Sandbox entitlement, hardened runtime, and exact
parent/helper identity before it can return `Enforced`.

The current native-app `app/macos/Runner/Release.entitlements` does not
enable `com.apple.security.app-sandbox`: the key is absent in the source
audited on 2026-10-30 UTC. It is incorrect to describe that source as an
explicit false-valued key. Gent must not treat that app process as a sandbox
parent. The separate helper entitlement enables App Sandbox, but its current
protocol only denies preparation; it does not launch a contained provider. This is not a request
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
