# Managed Windows provider resources

The contained Claude and Codex factories accept a managed platform package, not an arbitrary
developer executable. `ProviderExecutables::managed_runtime` verifies the current
signed ordinary-authority release, executable compatibility, exact canonical
managed location, durable package name/version/SRI provenance, and the release's
Node binding. A run's local metadata labels do not replace its executable identity.

The fixed `runtime/windows/provider-resources.json` file is authenticated by the
daemon build's `GENT_WINDOWS_PROVIDER_RESOURCES_SHA256` pin. Its version-one
`providers` list identifies the provider, Windows npm platform, exact package
coordinates, Node digest, and reviewed credential transport. Missing or mismatched
material leaves that factory unavailable. Computing a digest of the current files
does not establish trust or create this release artifact.

Each `files` entry has `root`, `path`, and `sha256`. Roots are `nodeRuntime`
(`runtime/node`), `package` (the verified managed package), or `gitRuntime`
(`runtime/windows/git`). Paths are bounded relative file paths with no traversal,
drive, alternate stream, or alias syntax. Runtime resources grant those exact
files. Node, the npm shim and npm CLI module, and the reviewed `usr/bin/bash.exe`
must be present. Every required additional runtime module or Git dependency must
be explicitly supplied by the reviewed artifact. The native command-line bound
also applies; exceeding it fails before launch.

`osLibraries` contains only a finite set of DLL basenames. The reviewed helper
resolves the real Windows system directory and proves Microsoft Windows embedded
or catalog provenance for protected OS files. The daemon retains deny-write/delete
handles and compares the helper's nonce-bound paths, measured hashes, and OS
identity. This permits authenticated OS updates without granting System32 as a
directory or trusting unsigned replacement libraries.

`credentialContract: "claude-loopback-v1"` authorizes the existing reviewed,
tool-disabled native credential transport for that exact managed package. Real
credentials remain in the owner and egress broker. The contained provider receives
an isolated state directory and an ephemeral gateway credential. The former
development-image gate remains only in explicit test code.

Claurst uses `credentialContract: "claurst-daemon-model-v1"` and an exact
`executableSha256`, with its own reviewed image and finite runtime closure. It does
not inherit Claude's account or Node authority.

The owner-side model runtime has a separate build-pinned
`GENT_WINDOWS_LLAMA_RUNTIME_MANIFEST` JSON contract: version one, exact Windows
platform, an executable basename, a finite `files` list of relative paths and
SHA-256 digests, and finite OS DLL basenames. The executable must occur in that
file list. Validation rejects malformed identities, path aliases, case-insensitive
duplicates, missing images and oversized closures before opening resources or
invoking OS inspection. Every model dependency remains locked against replacement
for the owned model lifetime. This contract does not grant the contained agent
access to the model files or owner credentials.

Contained Claurst currently rejects configured MCP servers because the required
reviewed MCP executable and IPC grants are not implemented. The model build pin
and reviewed model dependency artifact must also be supplied by release tooling;
validating a fixture does not supply those artifacts or establish provider support.
The six `windows_model_resources` regressions pass on Windows (2026-10-30),
including real temporary-file replacement/deletion denial while a resource pin is
held and changed-content rejection after release. These tests do not launch a
model, a contained provider, or an account transport.

Codex uses `credentialContract: "codex-file-responses-v1"`. Its pinned native
auth-only process supplies effective configuration and public account identity.
The owner reads the exact native-selected File store and keeps real credentials
inside the Responses egress broker. The contained child receives only an isolated
home and an ephemeral loopback credential. Keyring, external authentication,
custom origins, ambient credential overrides, and unsupported account modes fail
closed. This contract follows the pinned Codex 0.145 native storage and provider
configuration interfaces; no live account compatibility has been established.

The descriptor is preparation authority. Registry checks cannot produce live
`ProcessSandboxEvidence`; that evidence still comes only from an actually
contained process with its exact run, executable, policy, and host binding. File
handles remain owned by the routed process through its shutdown. The complete
helper proof allowlist remains unchanged and closed. Source fixtures and parser
tests do not establish production containment or account transport compatibility.
