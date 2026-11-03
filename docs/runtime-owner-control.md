# Runtime owner control

`runtime-owner-control-v1` is a local IPC capability for the composed standalone
authority. It does not authorize runtime downloads, replacement, process lookup,
restart, provider login, or prompt replay.

`runtimeOwnerStatus {requestId}` returns `runtimeOwnerState` with the same request
ID, `hostEpoch`, `processId`, `executableDigestSha256`, and `quiesced`. The digest
identifies the running executable, not the contents of a replacement archive.

`quiesceRuntimeOwner` requires `requestId`, `expectedHostEpoch`,
`expectedProcessId`, and `expectedExecutableDigestSha256`. The daemon rejects a
stale tuple. It also rejects recovery, active admission leases, native lifecycle
work, active durable dispatches, and pending permissions. Rejection preserves
open admission. No decision, message, receipt, target binding, or history is
rewritten.

Successful admission closes the same transient gate used by native jobs and
prompt requests and closes durable ingress in a transaction. Ordinary CLI
dispatches cannot claim pending work after closure. Existing queued inputs keep
their original disposition and bindings. Startup metadata work may hold counted
operation leases during recovery; prompt admission remains closed until recovery
finishes.

The response is `runtimeOwnerQuiesced`, containing the request ID and exact owner
tuple. The daemon flushes this response before authorizing its process to exit.
It completes its existing owner-proven lifecycle shutdown before returning from
bootstrap. A lost response leaves the owner quiesced; another request with the
same owner tuple can obtain and flush the acknowledgement. A stale request cannot
stop a successor.

The client must prepare and verify the trusted replacement before requesting
quiesce, then wait for the exact owner to exit before activating files used by
that owner. Receipt of `runtimeOwnerQuiesced` alone is not proof of process exit.
The reported PID is never authority for killing another process. New clients
must fail closed when the capability is unavailable; they must not substitute a
PID scan, kill-by-name, targetless shutdown, or a check-then-kill sequence.

Native auth and model probes retain an admission lease until their child process
has actually exited, including asynchronous kill/reap. Downloads, side questions,
prompt wakes, and local IPC exchanges retain leases through their owned work.
If reaping cannot be scheduled, the lease remains held and repair remains busy.

The trusted staged CLI exposes `runtime activation-capabilities`, returning
schema 1 with `runtime-activation-lease-v1` and `runtime-immutable-release-v1`.
After artifact verification, `runtime bootstrap-identity --bootstrap-dir STAGE`
returns the daemon digest and `bootstrapTreeSha256` for snapshot binding. A hash
of untrusted files does not establish artifact authority.

Repair invokes `runtime activate --bootstrap-dir STAGE --idle-data-dir DATA
--expected-host-epoch E --expected-daemon-sha256 SHA
--expected-bootstrap-tree-sha256 TREE --json`. Activation holds DATA/gentd.lock
and then the runtime store's .install.lock before publishing or selecting files.
All activation paths acquire both leases, including default-directory calls.
An acknowledged owner epoch must still match closed durable ingress. Changed
staged content, a competing owner, or a newer selected version fails before
publication. The machine result includes schema 1, canonical immutable daemonPath,
both digests, selectedRelease, dataDir, and originHostEpoch.

Generations use `vM.m.p+<tree SHA first 16>.<random 8 hex>-<target>`. Existing
generations are never restaged or overwritten; equal current generations may be
reused after whole-tree verification. Other data-directory owners retain their
old immutable files. A competing change to shared launcher files fails boundedly
if Windows holds those files open; repair does not kill other owners. Clients
launch the exact returned generation and verify the successor epoch and digest.
