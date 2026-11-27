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
change relists without a refresh (`model_catalog_refresh_tests.rs`). A refresh lists only what the
running provider binary reports. It also fetches the published provider authority (below), which
changes which provider binary Gent runs, never the listed ids.

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

## Updating models without an app or Gent release

Claude and Codex models change with the provider version, and the provider version is pinned by
the signed ordinary provider authority (`ordinary-authority.json`, `docs/releases.md`). It carries,
per target, the provider version and executable sha256 (compatibility) and the npm package and
SHA-512 integrity (package policy), signed by the runtime-release root whose public key ships in
every Gent archive (`platform/authority/root-keys.json`). Gentd accepts a newer one from the
`provider-authority` GitHub release (`published_provider_authority.rs`):

- At startup and on `readModelCatalog {refresh: true}`, Gentd fetches
  `releases/download/provider-authority/ordinary-authority.json` in the background. Nothing waits
  for it; a provider launches from the current authority meanwhile.
- The document must verify exactly as the packaged one does (`load_bound`: root signature, nested
  signatures, expiry, revocation, the installed Node runtime). It is staged as
  `<data dir>/provider-authority/published.json` only if it expires later than the active
  authority and any staged one, so a replayed older document is refused.
- Every failure is a typed `PublishedAuthorityError` (`Fetch`, `TooLarge`, `Refused`, `Store`)
  written to the daemon's stderr log; the active authority stays.
- On the next start Gentd adopts the staged document as `provider-authority/ordinary-authority.json`
  and runs on whichever of it and the packaged authority expires later and still verifies. One run
  uses one authority, because its provisioning, compatibility and review digests are composed
  from the authority verified at startup. The Linux sandbox document stays bound to the packaged
  authority.
- An installation the new authority does not cover becomes `InvalidInstallation` and its next
  prompt offers the reinstall, which installs the newly pinned package through the existing
  provisioning path. The catalog then relists from the new binary.
- An explicit `--standalone-authority-release` (development) never fetches.

To publish a pin: bump `pins.json` with the provider update playbook (§3 steps 1 to 8), merge it,
then run the `provider authority` workflow. It signs the authority from `pins.json` with the
release secret, carrying the Node runtime digests of the latest Gent release and of the
previously published authority, and replaces the `provider-authority` release asset. No Gent,
gentd or app build is involved. `crates/gentd/provider-models.json` stays a pre-listing fallback
only; the new ids come from the provider binary itself.

Local curated models change with `crates/gentd/models.json` and therefore with a Gent runtime
release. User-picked GGUF files need no release (`docs/local-model-files.md`).
