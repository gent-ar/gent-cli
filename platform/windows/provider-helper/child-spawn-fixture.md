Compile child_spawn_probe.cpp with the normal x64 MSVC toolchain and /W4 /WX
/O2 /MT /std:c++17. It accepts two absolute executable paths. For each it opens
read/execute access, creates a suspended child with ordinary and redirected
standard handles, then terminates and waits for the child. It never resumes a
child or sends a provider request.

Run it through the normal helper run command with reviewed resource leaves and
the probe pinned by SHA256, fresh owned writable workspace/state, no gateway,
and both targets after --. The first target is a reviewed leaf; the second is
an identical image in the parent of another reviewed leaf but absent from read
grants. Keep the second file outside writable roots.

Require both first-target rows: open=0 pipe=0 spawn=0 terminated=1. Require both
second-target rows: open=5 spawn=5 terminated=0. Require helper exit zero and
the enclosing owned Job Active0. The negative checks content and execution
authority despite the parent metadata grant.

On 2026-10-30, Codex 0.153.4's signed code-mode host returned spawn error 5 with
leaf-only policy. PSEC 1.1 parent fs_enumerate allowed the same image to spawn
while preserving ungranted-sibling denial. Directory-content access also
succeeded but was not selected as the correction. PAR-007 records evidence.
