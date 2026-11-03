# Permission mode release blocker

Audited against bundled v0.2.4 on 2026-10-25.

Autonomous and Bypass require daemon-verified OS containment in
`PermissionMode::requires_sandbox` and `evaluate_permission_with_sandbox`.
Production availability remains closed: Windows requires an accepted helper
pin and complete-policy proof, while non-Windows composition rejects the
required backend. Request preflight accepts only matching live process
evidence; source fixtures cannot supply production authority. Previously the
policy API acknowledged these modes, but every operation still asked for
approval. The API now rejects selecting them with an explicit explanation.
This is an interim correctness fix, not implementation of these modes.

The app now updates the displayed selection only after owner acknowledgment.
 Sending a prompt refreshes the selection from the owner workspace
policy instead of writing a potentially stale conversation setting over it.
Legacy persisted broad policies are revalidated before sending; rejection reports
the error and stops submission. Selecting Ask or Auto-accept edits remains possible.

## Implemented boundary and remaining backends

`gent-drivers::SandboxedProviderLaunch` owns atomic containment, executable-lock
verification and process spawn. `WindowsSandboxLauncher` is a native
implementation with scoped PSEC/Job and packaged-proxy evidence; describing
the port as fixture-only is stale. `SystemLauncher` deliberately does not
implement it. The macOS helper module is a denial-only protocol edge, and
there is no Linux contained launcher in the daemon composition.

Completion requires platform implementations that prove containment of the exact
locked executable and its descendant process tree, enforce the configured workspace,
environment, network and resource boundaries, and supply the resulting trusted
enforcement state to permission preflight for that run. Windows still needs
accepted final packaging/composition and full escape/failure/restart/provider
acceptance; macOS and Linux still need actual native launch backends and
platform-native tests. Android is currently a remote companion rather than a
local agent host; its selected desktop owner's containment gate remains
authoritative. A provider setting
or a client-supplied boolean cannot establish this trust. Codex sandbox options
currently passed to its protocol are not daemon-owned launch attestation and do
not meet the existing shared contract on their own.

## Required verification before enabling broad modes

- Real contained provider launches and descendant processes on each host OS.
- Denied writes, reads, egress and inherited credentials outside the profile.
- Rejection on executable changes, failed setup, unsupported hosts and lost attestation.
- Local and paired-client selection, reconnect and simultaneous conversation changes.
- Claude, Codex and ACP operations evaluated against the same current workspace policy.

Do not mark broad-mode support ready based on the policy-save rejection tests.

## Historical Windows implementation plan

The following plan records the pre-native-backend audit. It is not the
current implementation inventory: `WindowsSandboxLauncher` now implements
the contained launch port, and ordinary `SystemLauncher` process handling
still must not be mistaken for that backend. Current production gates and
platform status are described above and in `sandboxing.md`.

Source audit: `process.rs` launches through `SystemLauncher`; its Windows
`configure_process_tree` is a no-op. `process_windows.rs` implements termination,
not containment. Neither can honestly produce `SandboxEnforcement::Enforced`.

Implement a Windows `SandboxedProviderLaunch` backed by an AppContainer/LPAC
process token, explicit resource grants and a supervised process tree. Windows
requires an AppContainer SID and capability attributes at process creation;
access also depends on the resource DACL. See Microsoft's
[AppContainer launch contract](https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer).
This is a proposed backend, not an implemented or validated compatibility claim.

Before implementation, expand the launch profile from workspace roots and a
network-policy digest into enforceable grants: provider/runtime executables and
libraries, read-only configuration, minimal authentication access, writable
workspace/temp/cache locations, and an explicit egress policy. Inventory Claude,
Codex and ACP child-process requirements using isolated fixtures. Provider login
and arbitrary host credential access must not be implicitly granted together.

Create the process suspended with its security attributes and explicit inherited
handles, attach supervision, verify the actual token/profile and exact locked
executable, then permit execution. A kill-on-close job handles descendants and
cleanup but does not replace token/resource isolation: Microsoft's
[job-object documentation](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
requires security limits on each process. Test child spawning, handle inheritance,
breakaway attempts, reparse-point escapes, profile setup failure, credential reads,
blocked network access and owner restart before enabling broad modes.

The initial deliverable should be a deny-by-default fixture launcher with positive
and negative filesystem/network probes, followed by real provider compatibility
tests. Only successful atomic launch attestation for the current run may flow into
permission preflight. This is substantive native runtime work; changing the UI,
adding a Job Object alone, or trusting provider flags does not complete PAR-007.
