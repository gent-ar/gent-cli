# Model catalog

The model picker renders `model-catalog-v1` (`readModelCatalog`), served by
`crates/gentd/src/model_catalog_service.rs`. Gentd keeps one slot per provider and fills it from
that provider's own source. Nothing in it is a Gent-maintained list of Claude or Codex models.

## Where each provider's models come from

| Provider | Source | Code |
|----------|--------|------|
| Gent local (Claurst) | Curated GGUF catalogue shipped in `crates/gentd/models.json`, plus user-picked files | `model_catalog_local.rs`, `local_model_catalog.rs` |
| Claude | `models` in the pinned Claude Code's `control_request initialize` response | `model_catalog_claude.rs`, `model_catalog_claude_initialize.rs` |
| Codex | `model/list` (all pages, `includeHidden: false`) from the pinned Codex `app-server` | `model_catalog_codex.rs` |

Claude and Codex are probed with the exact driver argv, so the list is what the provider binary
Gent runs would accept. Only while a provider has not listed (not installed, signed out, probe
failed) does the slot show its last live listing (`<data dir>/model-catalog-last-known.json`),
and only without one the static `crates/gentd/provider-models.json`, which holds ids the pinned
binary listed (`model_catalog_provider_models_tests.rs`).

## Refresh

`readModelCatalog {refresh: true}` relaunches every provider probe at once, even inside the
600 s TTL, and replies immediately with those slots `listing: loading`. A later read returns the
settled listing. Refreshes during a probe coalesce into one more probe, and a provider executable
change relists without a refresh (`model_catalog_refresh_tests.rs`). A refresh can only show what
the running provider binary reports; it never reads a remote Gent document.

## Why Codex stops at the pinned version's models

Codex filters its catalog by client version. Each Codex build embeds a model list whose entries
carry `minimal_client_version`, and its remote `/models` fetch sends `client_version`. Observed
2026-11-26 on Windows:

- Pinned `codex-cli 0.153.4` (npm integrity matches `pins.json`) embeds `gpt-5.2`, `gpt-5.4`,
  `gpt-5.4-mini`, `gpt-5.5`, `gpt-5.6-{sol,terra,luna}` and `gpt-6-astra`.
- `codex-cli 0.170.0` `model/list` on the same account returns `gpt-6.2-sol` (default),
  `gpt-6.2-astra`, `gpt-6.1-terra`, `gpt-6.1-sol`, `gpt-6-astra`, `gpt-6-sol`, `gpt-6-luna`
  and `gpt-5.6-{sol,terra,luna}`.

Newer models therefore appear in Gent only when Gent runs a newer Codex. A remote Gent model
list would offer ids the pinned client does not list and may refuse, so Gent does not have one.

## Updating models without an app release

Claude and Codex models update with the provider version. Bump the pin with the provider update
playbook (`docs/provider-update-playbook.md` §3) and ship it in a signed Gent runtime release.
Installed runtimes pick it up through the runtime update path (`docs/releases.md`); the Flutter
app needs no rebuild because it renders whatever the catalog lists. After the new provider
binary is active, the catalog relists on its own; Refresh only shortens the TTL wait.

Local curated models change with `crates/gentd/models.json` and therefore with a Gent runtime
release too. User-picked GGUF files need no release (`docs/local-model-files.md`).
