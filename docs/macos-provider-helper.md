# macOS provider-helper release foundation

`GentProviderHelper.app` is a Gent-owned, independently packaged helper for
the future macOS provider-launch boundary. Packaging it does **not** enable
provider authority or claim containment.

## Fail-closed protocol foundation

The helper supports `--version` and a one-request `--protocol` stdin/stdout
JSON boundary. Protocol version 1 accepts only a bounded `prepare` request:
the exact helper bundle/version identity, a structural Claude/Codex immutable
lock, a profile digest/network posture/resource limits, and an optional
security-scoped workspace bookmark. It has no argument vector, command,
shell, `PATH`, prompt, or output field.

The helper rejects unknown JSON fields, malformed identities, noncanonical
provider paths, invalid digests, unsupported providers, and invalid resource
limits. It resolves an offered security-scoped bookmark without UI; an absent,
stale, or inaccessible bookmark receives an explicit denial. Even with a
valid bookmark, every request currently returns
`containmentSemanticsUnavailable`: this foundation never launches a provider
or reports enforcement. `Process`/`NSTask` are prohibited by its test.

The response is also bounded, contains only protocol/request/helper identity
and a coarse result code, and cannot return source paths, command data,
environment values, provider output, or credentials. A future launch revision
requires separate review, containment integration tests, and a new protocol
version; it must not weaken this fail-closed behavior.

`gent-drivers` has a typed, bounded Rust client for this exact protocol. It
accepts a reviewed one-request transport and validates the response identity,
shape, request id, and explicit denial. It deliberately has no process
transport and does not implement the contained-launch port, so using it cannot
enable provider authority.

The native app’s release entitlement was inspected read-only on 2026-08-18.
It is not App Sandbox enabled, so Gent does not inherit or reuse it. This
bundle has the minimum current entitlement: App Sandbox enabled, with no
network, filesystem, JIT, or child-process entitlement. Any future expansion
requires a separate review, a helper protocol, and platform evidence.

## Local signed build

The Developer ID certificate and private key stay in the local Keychain. Do
not export a `.p12`, commit a certificate, pass a password, or put a signing
secret in an environment variable. Supply only the public identity label or
its Keychain fingerprint and the expected team identifier:

```sh
export GENT_MACOS_SIGNING_IDENTITY='Developer ID Application: Ivan Fort (G92HDH3SF5)'
export GENT_MACOS_SIGNING_TEAM_ID=G92HDH3SF5
python3 tools/build-macos-provider-helper.py
```

The builder rejects non-Developer-ID identities, builds into the ignored
`target/macos-provider-helper/` directory, signs with hardened runtime and a
secure timestamp, then runs the verifier. An explicit output must end in
`.app`:

```sh
python3 tools/build-macos-provider-helper.py \
  --identity 9FEC8C02CB712CF3C70668EE35C7BFEB14F6BE8B \
  --expected-team-id G92HDH3SF5 \
  --output target/macos-provider-helper/GentProviderHelper.app
```

Verify a previously built bundle without signing it:

```sh
python3 tools/verify-macos-provider-helper.py \
  target/macos-provider-helper/GentProviderHelper.app \
  --expected-team-id G92HDH3SF5
```

The verifier requires the exact bundle id/executable, Developer ID authority,
team identifier, hardened-runtime flag, strict deep signature verification,
and exactly the committed App Sandbox entitlement.

## Notarization

Notarize only a release candidate after signature verification. Store Apple
notary credentials in a local Keychain profile created with
`xcrun notarytool store-credentials`; provide its profile name to
`xcrun notarytool submit --keychain-profile ... --wait`. Never place an Apple
ID password, app-specific password, API private key, or profile contents in
this repository, shell history, CI logs, transcript corpus, or environment.
After an accepted submission, staple and assess the exact packaged app with
`xcrun stapler staple` and `spctl --assess --type execute --verbose`.

Notarization alone is insufficient. Before macOS provider authority exists,
Gent still needs the reviewed helper IPC, immutable lock/profile-bound launch
attestation, bounded child-tree control, and negative integration evidence
listed in [sandboxing.md](sandboxing.md).
