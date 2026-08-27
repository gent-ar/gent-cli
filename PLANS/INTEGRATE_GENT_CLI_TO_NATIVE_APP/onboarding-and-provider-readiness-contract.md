# Onboarding and provider readiness contract

Paths are relative to `/Users/ivanmatiasfort/Clouseau/clouseau-app` unless prefixed `crates/` or
`tools/`, which are `/Users/ivanmatiasfort/Clouseau/gent-cli`.

## Ownership

Gentd owns provider selection, the local model catalog, model installation state, public-provider
installation and provider readiness. Both clients render the same state and request actions. Neither
persists readiness nor performs an independent provider installation.

## Desktop distribution

The Gent release archive is already the complete runtime set. `tools/package-release.py` produces:

```
gent-{version}-{target}/gent
gent-{version}-{target}/gentd
gent-{version}-{target}/gent-launcher.exe          (Windows only)
gent-{version}-{target}/runtime/node/bin/{node,npm}
gent-{version}-{target}/runtime/node/lib/node_modules/npm/bin/npm-cli.js
gent-{version}-{target}/runtime/claurst/claurst
gent-{version}-{target}/runtime/claurst/llama/llama-server
```

llama.cpp already ships inside it. No component needs to be added to the archive.

| Component | Distribution | When it runs or downloads |
| --- | --- | --- |
| `gent` | In the archive; currently dropped during staging | Only when the user invokes the terminal or the installed command shim |
| `gentd` | In the archive; staged today | The app starts one resident daemon; `gent` connects to the same daemon and directory |
| Node and npm | `runtime/node/` | Only when Gentd installs or runs a public provider or a Node MCP |
| Claurst and llama.cpp | `runtime/claurst/` | Started only for an accepted local-model prompt; stopped when no runtime work remains |
| Curated model weights | Never in the installer | Downloaded after an accepted prompt selects the model; persisted in the Gent data directory for both clients |
| Claude Code and Codex CLI | Never in the installer | Installed into Gent's managed Node prefix when an accepted prompt selects the provider and the executable is absent |

The native build must stage that archive intact. Today it does the opposite:
`tools/stage-gentd.py:129-133` keeps only `gentd` and `runtime/`, dropping `gent` and
`gent-launcher.exe`, and L141-148 overwrites the archive's `runtime/node` and `runtime/claurst` with
the app's own separately downloaded copies pinned by four independent `mate.json` `bundled` versions.
Backlog item 6 removes both behaviors and collapses `mate.json` to one `gentRuntime` entry. The
installed app then exposes `gent` through its installer-owned command shim; the Flutter UI still
never spawns the terminal.

## Native source map

| Native concern | Current source | Gentd contract |
| --- | --- | --- |
| Provider and model selection | `app/lib/provider/agent_chat_provider.dart`, `app/lib/widget/agent_chat/input_chips_row.dart`, `app/lib/widget/agent_chat/panel_composer_models.dart` | Current selection, `provider`/`model` catalogs, capability flags, immutable context-preserving switch intent |
| Local model install | `app/lib/service/gentd/gentd_ipc_client.dart`, `app/lib/provider/agent_chat/agent_chat_gentd.dart` | `local-models-v1`, keyed by model ID **and** request ID |
| Local model presentation | `app/lib/widget/agent_chat/thinking_indicator.dart`, `app/lib/widget/agent_chat/input_bar.dart` | Pending prompt and download progress on one ordered activity stream |
| Public provider install | `app/lib/provider/agent_chat/agent_chat_adapters.dart`, `app/lib/provider/agent_chat_provider.dart` | An accepted Claude/Codex prompt prepares the managed install and reports preparing, ready or a safe failure |
| Public provider login | `app/lib/widget/agent_chat/panel_dialogs.dart`, adapter launch paths | `provider-auth-v1` action descriptor, lifecycle, and post-return readiness refresh |
| Prompt admission | `app/lib/provider/agent_chat/agent_chat_send_dispatch.dart`, `app/lib/provider/agent_chat/agent_chat_gentd.dart` | The prompt is durable before preparation begins and released exactly once when readiness becomes ready |

## First-use flow

1. A new conversation defaults to Claurst with the curated default local model.
2. The first prompt persists. If the model is absent, Gentd starts one managed download and publishes
   ordered progress for that prompt.
3. The client shows the download in its working indicator and offers cancel. Cancel detaches that
   prompt's admission; it never cancels another conversation's wait for the same shared model. Gentd
   cancels the underlying download only when no admission remains, or on an explicit model-download
   cancellation intent. Completion starts the local runtime only for an admitted prompt.
4. Switching to Claude or Codex changes selection only and installs nothing. An accepted prompt
   installs the selected managed CLI if it is missing.
5. If the installed provider needs authentication, the client presents one login action. Gentd starts
   the provider's own interactive login and the client refreshes readiness on return.
6. The original durable prompt proceeds after login without the user retyping it.

## Projection status

| Projection | Exists today | Work required |
| --- | --- | --- |
| `providerCatalog` | No | Built by backlog item 2 as the `provider` and `model` catalogs. Effort and mode are separate catalogs referenced by ID, not fields on the provider record. |
| `providerReadiness` | `provider-readiness-v1` is declared in `crates/gent-runtime/src/catalog.rs` | Audit its fields against: conversation, run, provider, phase, reason category, retryable, pending prompt count |
| `modelDownload` | `local-models-v1` is declared | Add `requestId`. Model ID alone cannot say which indicator or cancel control owns an admission when concurrent prompts select the same model. |
| `providerInstall` | No | Add: request ID, provider, phase, pending prompt count, safe failure category |
| `providerLogin` | `provider-auth-v1` is **defined but never declared** — the constant exists at `crates/gent-protocol/src/provider_auth.rs:15` and the handler gates on it at `crates/gentd/src/provider_auth_transport.rs:27`, but it is absent from `crates/gent-runtime/src/catalog.rs`, so negotiation can never reach it | Declare it, then add: request ID, provider, phase, external route, return status, safe failure category |
| `promptAdmission` | No | Add: prompt receipt, conversation, run, phase, release or failure reason |

Every install, download and auth lifecycle carries its own request ID and prompt receipt. A client
cancellation affects only that receipt unless Gentd reports the shared operation has no remaining
waiters.

## Native authentication handoff

Provider authentication is a daemon-issued action descriptor, not a Flutter provider implementation.
The descriptor may request opening an external URL, displaying a device code, receiving a completion
callback, or opening a provider-owned terminal handoff when that CLI supports nothing else. Flutter
renders it generically and returns the receipt. Credentials, CLI arguments and readiness verification
stay inside Gentd.

## Current standalone behavior

- Gent defaults to Claurst with the curated local model and starts no llama.cpp process until a
  prompt needs one.
- A missing local model downloads automatically, with progress on the local-model event stream.
- Claude and Codex install into Gent's managed Node prefix only when a selected prompt needs them.
- `gent auth login claude`, `gent auth login codex` and terminal `/login` invoke the provider's own
  login interface. Gent holds no credentials. `/login` currently starts a separate one-off `gentd`
  process because `provider-auth-v1` is never advertised; that workaround is deleted once the
  capability is declared.

## Deferred Flutter integration

1. Replace adapter-owned install and readiness state with the projections above.
2. Keep draft text, selected pane, scroll position and modal state in Flutter.
3. Render the external-login handoff from `providerLogin`; embed no credentials in IPC or Dart
   persistence.
4. Reuse the same prompt admission receipt when the user returns from login.
