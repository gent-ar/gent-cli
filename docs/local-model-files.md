# Local model catalog and user-picked GGUF files

Gent's local models (Claurst over llama.cpp) come from two sources that share one record type,
one listing and one launch path:

- **Curated**: the compiled-in `crates/gentd/models.json` records, downloaded on request from a
  pinned Hugging Face revision into `<data dir>/models/<id>/` and verified by SHA-256.
- **User file**: a `.gguf` the user picked. Gentd references the file in place (it never copies,
  moves or deletes it) and persists one entry in `<data dir>/user-models.json`.

Gentd is the only writer of `user-models.json`. A malformed registry lists no user models and
refuses add/remove instead of overwriting the file. Entries are never removed automatically.

## Wire contract

Both frames ride the existing `model-catalog-v1` envelope and are accepted only when the client
also negotiates `local-model-files-v1` (advertised whenever local models are enabled). The reply
to both is the usual `modelCatalog` frame, read after the local listing is reloaded, so the added
or removed model is already reflected.

```json
{"type":"addLocalModel","body":{"requestId":"add-1","path":"C:\\Users\\me\\Models\\m.gguf"}}
{"type":"removeLocalModel","body":{"requestId":"remove-1","modelId":"user-my-model-q4-3f2a91c0"}}
```

`path` is an absolute path (at most 4096 bytes, no control characters, no surrounding space).
Adding hashes the whole file before it replies, so a client must allow minutes, not the usual unary
timeout, for a multi-gigabyte model. Each request uses its own connection, so nothing else waits.

`LocalModelAvailability` (the `local` object of a catalog model) gains two required fields:

```json
{"sizeBytes":4368439584,"install":{"state":"missing"},"removable":true,"fileName":"m.gguf"}
```

- `removable` is `true` exactly for user files, and then `fileName` is the file's name. The
  directory and full path never leave Gentd: not in catalogs, projections or remote relays.
- `install.state` adds `missing`: a user file that no longer exists or whose size or modification
  time changed since it was added. Curated models never report it.

A client that does not negotiate `local-model-files-v1` never sees user models in catalog replies
(so it never meets `missing`), and its add/remove frames are not handled.

Rejections are ordinary `error` frames; match `code`, never `message`:

| code | meaning |
|---|---|
| `localModelNotGguf` | not a `.gguf`, bad magic, version other than 2/3, truncated or malformed metadata, no `general.architecture` or `<arch>.context_length` |
| `localModelMissingChatTemplate` | no non-empty `tokenizer.chat_template` |
| `localModelContextTooSmall` | `<arch>.context_length` below 4096 |
| `localModelUnsupportedPath` | relative, network, device or UNC path; a symlink; a directory; on Windows any reparse point in the path |
| `localModelUnreadable` | missing or unreadable file, or it changed while it was being hashed |
| `localModelNotRemovable` | `removeLocalModel` named a curated or unknown model |
| `modelCatalogRejected` | registry storage failure (message is diagnostic only) |

`localModelDownloadFailure` adds `modelFileMissing`, the admission failure for a prompt whose
user model is missing.

## Derived record

Gentd reads only the GGUF header and metadata, with every length and count bounded by the file
size and a 256 MiB metadata cap; it never allocates from an untrusted count and keeps at most
4 KiB of any string. It needs no architecture allow-list: an architecture llama.cpp cannot load
fails as the existing typed launch or readiness failure.

- `id`: `user-` + a slug of the label (`[a-z0-9-]`, at most 48 bytes) + `-` + the first 8 hex
  digits of SHA-256 of the canonical path. Re-adding the same file replaces its entry.
- `label`: `general.name`, else the file stem.
- `contextTokens`: `<arch>.context_length` clamped to 4096..=32768.
- `runtimeMemoryBytes`: 1.5 × size + a q8_0 KV-cache estimate (layers × context × KV heads ×
  key+value length × 17/16; half the size when the metadata lacks those keys) + 512 MiB.
- `agentProfile`: `compact` below 2.5 GB, otherwise `full`.
- No chat template file: llama.cpp renders the model's embedded template (`--jinja`).
- `sha256`: computed once when the file is added; size and modification time are re-checked on
  every listing, selection and launch.

## Launch

The llama-server argv is generated from the record exactly as for curated models: `-m <canonical
path>`, `--ctx-size <contextTokens>`, `--jinja`, the q8_0 cache flags and `--parallel 1`. User
models never get `--chat-template-file`, `--chat-template-kwargs` or the Qwen3 reasoning flags
(those key on curated `qwen3-` ids), and never take user-supplied arguments or environment.

A moved, deleted or modified file is reported three ways, all typed: the listing shows `missing`,
`createConversation`/`switchSelection` fail `selectionModelUnavailable`, and the launch
re-validates immediately before spawning llama-server (`ModelFileMissing`). Adding the file again
refreshes the entry.

On Windows the contained model actor pins the file in place: it opens the canonical path without
following reparse points, holds the handle with read-only sharing for the life of the run,
re-verifies the SHA-256 through that handle and grants the sandbox read access to that one path.
It has no data-directory residency requirement, so nothing is copied.

## Code map

`gent-protocol::model_catalog` (frames, `LocalModelFileRejection`, capability);
`gentd::local_model_provisioning` with its children `local_model_gguf.rs` (header parser),
`local_model_user_file.rs` (path rules, derivation, identity) and `local_model_user_registry.rs`
(persistence, ids); `model_catalog_transport.rs` (dispatch, capability gating, typed errors);
`model_catalog_local.rs` (listing); `model_catalog_selection.rs` (validation and the uncached
local reload); `claurst_local_readiness.rs` (launch re-validation).
