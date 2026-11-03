# Linux cgroup2 ownership prototype

Actual safe Rust (no unsafe code) using retained cgroup directory descriptors,
openat2 without symlinks, real filesystem identity checks and a private new scope.
The parent must already be delegated to an unprivileged, capability-free caller;
the library never configures ancestors, creates delegation or accepts root.

Before a helper enters, the scope writes and verifies memory.max, zero swap,
memory.oom.group, pids.max and one-core CPU bandwidth. It requires tree-kill/events
support. A retained Child, parent PID, UID and process birth are checked before
migration. The helper must be a pinned trusted image blocked on its control pipe;
the caller releases it only after verified membership. MXC descendants inherit
that scope. This API does not itself establish helper-image trust.

Cleanup kills only the retained scope, waits for populated0, validates directory
identity and removes the exact owned name. It never recursively deletes arbitrary
groups. Initialization failures clean an empty owned group; incomplete cleanup is
an error/quarantine condition. Drop makes a bounded best-effort cleanup attempt.

## Actual evidence

Build0/source drift0. Six unit/actual negative checks passed as UID65534; ordinary
directories, invalid paths and the host's root-owned hierarchy cannot masquerade
as delegation. No normal unprivileged delegation exists on this WSL host.

A separately authorized root fixture provisioned only a fresh bounded QA subtree
under the already configured user@0.service, with no ancestor controller changes,
accounts or existing-process migration. The actual library and helpers ran as
unused numeric UID59991, no supplementary groups/capabilities, NoNewPrivs1.

- A six-fork positive control succeeded outside the worker group. Within pids.max4,
  fork failed with EAGAIN and pids.events.max incremented.
- A64MiB positive allocation succeeded outside. Within memory.max32MiB,96MiB
  allocation was killed by the memory controller (memory.events.oom_kill2).
- Aggregate CPU usage crossed the50,000µs monitored budget at54,185µs; cgroup.kill
  settled it at57,819µs. The7,819µs overshoot is explicit (final hardened build).
- The actual typed MXC fixture ran successfully after blocked-helper admission;
  the inherited scope observed peak7 tasks. Each worker scope was emptied/removed.

Final preflight also rejects mismatched saved/filesystem UIDs and any effective,
permitted or ambient capability, rather than checking effective UID alone.
The root fixture then removed its entire QA subtree. Ancestor policy/ownership and
the root coordinator's membership stayed unchanged. Independent later verification
found all three attempt paths absent and zero processes under the numeric QA UID.
The first attempt failed closed before GO because a comparison omitted a leading
slash; that failure and successful cleanup remain preserved as r1 evidence.

## Production blockers

cpu.max limits bandwidth, not total CPU time. This prototype's CPU sampling is
monitored termination, not a strict kernel total-CPU ceiling or automatic watchdog.
No Gent ProcessSandboxEvidence is created and production admission stays rejected.
Deployment still needs a genuine user delegation, trusted per-run helper/control
pipe, retained executable/policy/lease pins, owner-crash recovery and journal,
gateway credential/MCP topology and tested interruption semantics. Normal Drop
cleanup does not constitute crash-recovery evidence. macOS is outside cgroup2.

Authoritative semantics:
https://docs.kernel.org/admin-guide/cgroup-v2.html#delegation
https://docs.kernel.org/admin-guide/cgroup-v2.html#core-interface-files
https://docs.kernel.org/admin-guide/cgroup-v2.html#cpu
