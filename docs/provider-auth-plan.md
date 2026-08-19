# Provider authentication plan

Provider authentication belongs to Gent's future public-driver authority, not
to the Flutter application. This plan establishes the contract now without
claiming that the observer daemon can launch a provider or authenticate a user.

## Product boundary

Gent detects usable Claude/Codex authentication only through the locked vendor
binary's documented status command. It never reads credential files, keychains,
browser profiles, environment values, or vendor configuration. Claurst remains
an app-private bridge and is excluded from this public contract.

`gent-ports` now defines this discovery boundary as a secret-free port returning
only the daemon-owned binary lock, authentication state, and exact-version
methods. The observer daemon does not construct or invoke any implementation.

If a locked executable reports that it is signed out, the future daemon emits a
durable, typed `askTool` authentication challenge. The terminal and a later
native-app IPC consumer render exactly the same choice. Neither consumer owns,
stores, copies, or relays credentials.

The terminal now exposes `gent auth status <claude|codex>` and `gent auth login
<claude|codex>` as negotiated, secret-free requests. A later interactive chat
surface maps `/login <claude|codex>` to the same request. The native app sends
that request through its long-lived `gentd` connection rather than spawning
Claude or Codex itself.

```text
ProviderAuthRequired {
  challenge_id, provider, binary_digest, methods, expires_at
}
```

The response names a method, never supplies an untyped command. Account browser,
device code, API key, and access-token choices are included only when the
specific locked vendor version documents them. The result carries only a typed
state: `openingBrowser`, `awaitingDeviceApproval`, `verifying`, `authenticated`,
`failed`, `cancelled`, or `timedOut`.

## Process and secret boundary

- The daemon rechecks executable path, canonical identity, version, and digest
  immediately before discovery and again immediately before login. Any change
  yields `providerChanged`; no process starts.
- Browser/device login is an explicit user response even in Autonomous or
  Bypass mode. Those modes never select an account or authenticate silently.
- Gent starts only the official locked provider command: Codex login or Claude
  auth login. It runs outside the workspace with no project-write access.
- Vendor CLIs retain their own credential/keychain storage. Gent records only
  method and terminal outcome; it never persists provider URLs, device codes,
  account identities, keys, access tokens, or provider output.
- An API key or access token, where supported, is an ephemeral secret written
  only to the child process's stdin. It never appears in argv, environment,
  SQLite, receipts, events, logs, crash reports, or fixtures; buffers are
  cleared immediately after launch.
- The authentication process needs a narrowly allowlisted vendor-auth network
  profile and browser/local-callback support. It is distinct from a workspace
  provider-run sandbox.

## Bundled-Node provider provisioning

The native app distributes a supported Node runtime with its installed Gent
pair, but it never distributes a Claude Code or Codex executable. It passes the
bundled executable through `GENT_NODE_BINARY` at host bootstrap. Gent
canonicalizes and identity-locks the explicit Node and sibling `npm` paths. It
does not discover a bundle path or infer an app runtime root, and owns every
subsequent process.

On the first prompt selecting a missing public provider, an approved Gent
authority may perform exactly one receipt-backed provisioning transaction using
fixed `npm --global install` arguments. Immediately before that effect, Gent
re-reads the exact durable accepted receipt, idempotency key, and host epoch;
a changed or unavailable receipt fails without running `npm`. The install
target is a private Gent provider prefix, never the app bundle, system-global
prefix, workspace, or
`PATH`; the resulting executable is rediscovered, version-probed, digest-locked,
and checked against a signed package/version/integrity compatibility policy
before it can authenticate or run. A successful provider update follows the
same transaction and creates a new immutable run lock. An interrupted or
ambiguous `npm` process is `unprovable`, not retried automatically.

The user prompt is the initiation point, not proof of package authority. Gent
must surface terms/consent required by the selected vendor/package policy and
record the decision durably. Observer mode, missing evidence, an unsigned
package policy, changed Node runtime, or a lock mismatch fail before `npm`
starts. Claurst is excluded: its private bridge never uses public `npm` policy.

The app-driver removal and terminal/native parity gate is
[native-app cutover readiness](native-app-cutover-readiness.md). Provisioning
alone never authorizes a provider launch or changes observer-mode capabilities.

## Authority and evidence gate

The current observer milestone must keep all login commands hard-disabled. The
types, pure reducer, protocol, test fakes, and terminal rendering may land
before authority, but `gentd` may compose the launcher only after:

1. OS sandbox enforcement exists for the requested platform and provider.
2. The provider binary is compatibility-authorized and digest-locked.
3. The public-driver evidence matrix includes redacted live login evidence for
   every supported provider and authentication method.
4. The command/output capability table is pinned to the observed vendor version.
5. The private Claurst bridge has its separate app-owned, private CI evidence.

Tests must prove signed-out discovery, typed choices, cancellation, expiry,
timeout, retry, unsupported methods, changed-binary refusal, and that no secret
reaches argv, environment, persistence, events, logs, or fixtures.

## Vendor references

Codex documents `codex login status`, browser login, device authentication, and
stdin-based API-key/access-token flows in its
[developer command reference](https://learn.chatgpt.com/docs/developer-commands#codex-login).
Claude documents `claude auth status` and `claude auth login` in its
[CLI reference](https://code.claude.com/docs/en/cli-usage) and
[authentication guide](https://code.claude.com/docs/en/authentication).
Gent remains a local launcher rather than a credential broker; Anthropic's
[legal guidance](https://code.claude.com/docs/en/legal-and-compliance) requires
additional product review before a native app presents Claude subscription login.
