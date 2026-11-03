# Linux companion archive and user-service delivery contract

This stage builds artifacts; it never installs packages, changes trust, creates a login/user manager, enables units or starts a service. No genuine signed-bundle acceptance or reusable workflow execution has been claimed.

## Inputs and trust

The trusted release job must supply a complete flat `bundle.tar` containing the SAME normal `gent`, `gentd`, Node, Claurst and ordinary authority it intends to publish, plus `authority/linux-sandbox.json` and all approved assets beneath `runtime/linux-sandbox/`. The companion must already carry a genuine signature accepted by committed ordinary release roots. The existing root signer remains the sole authority; this packaging workflow has no release key or signing secret.

The external production verifier must be independently authenticated by the trusted build/provenance chain before its digest is passed to the reusable workflow. A digest supplied by the candidate itself does not satisfy that requirement. Artifact digests are mandatory inputs from the trusted protected-tag caller. Root-owned temporary copies bind stream hash and inode/generation before parsing/execution. The production companion loader, exact shared normal component comparison and final tar hashes must all pass. All normal components, including gentd and Claurst, are checked again in the emitted archive.

`linux-contained-package.yml` is artifact-only and requires a protected semantic-version tag. It is deliberately NOT automatically wired into `release.yml` publication: the repository still lacks a genuine reproducible build/provenance job for the finite Node MCP bundles and network closure. It must not be replaced with invented fixture inputs, arbitrary download URLs or artifact-self-asserted verifier pins. Existing release.yml adds prerequisite tests without breaking default Linux/macOS/Windows packaging.

## Maintained distro package path

A user-owned tar extraction cannot meet immutable helper/network origin requirements. The separate Debian builder uses distro-maintained `dpkg-shlibdeps` for ELF dependency metadata and `dpkg-deb --root-owner-group` to encode root ownership. It creates `/usr/lib/gent/<version>` and `/usr/lib/systemd/user/gent-contained.service`; no maintainer scripts, daemon-reload, auto-enable, startup or trust modification are included. A genuine distro package still needs the normal release authentication/publication and an administrator's explicit installation. Dependency discovery for an actual Gent cohort remains untested until genuine inputs exist; the real public-system-ELF control does not establish Gent dependency completeness.

The user unit requires systemd>=254, a real nonroot user manager and existing cgroup2 nsdelegate delegation. DelegateSubgroup puts the main process into `controller`; the explicit reviewed startup method enables only memory/pids in that OWN already delegated empty service root. It cannot create delegation or remount/configure the host. Gent itself resolves its default profile/data directory; no profile path is copied into unit policy. This unit is opt-in and does not replace an existing installed GUI owner.

Current WSL read-only prerequisite check: PID1 is systemd259 but QA UID59991 has no user bus. No session was created; no actual user-systemd activation is proved. Prior service-shaped cgroup positive/negative controls remain separate evidence. Installing/starting this unit is outside this stage's authorization.

## Actual tests and remaining proof

`source-tests-r1/result.json` binds exact source bytes: syntax;6 existing producer controls;9 archive/CI-input consistency controls; legacy deterministic tar+zip regressions; actual dpkg metadata root ownership/no-maintainer-script control; actual maintained dependency resolver against a public system ELF. Consistency fixtures deliberately use invalid signature bytes and never invoke the production authority path. No placeholder or fixture is a release authorization.

Pending: independent source review, real authenticated Node-MCP/network producer inputs and protected caller wiring, legitimate ordinary+companion signed artifact, real production-verifier/archive/deb acceptance, actual nonroot user-systemd activation, native OAuth/normal registry positive and all four product modes. Ask/Auto retain existing host routes; new Linux containment remains Claude Autonomous/Bypass. CPU remains monitored total5ms, not a hard kernel total-CPU quota.
