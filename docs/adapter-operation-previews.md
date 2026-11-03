# Operation previews

Every adapter supplies operation details through the shared `ToolActivity.input`
contract and `PermissionRequest.input`; clients do not parse provider frames.
`ToolActivity.input` is optional so existing persisted facts continue to deserialize.
New adapters call `gent_types::operation_preview` when constructing activity facts.

The preview retains common operation fields such as `command`, `cwd`, `file_path`,
`content`, `old_string`, `new_string`, `changes`, `diff`, `query`, and `arguments`.
Provider-specific tools normalize their arguments into these fields. Transport
metadata and environment maps are excluded. Nested credential fields and common
credential tokens are redacted. The preview is bounded to 16 KiB with explicit
truncation markers, and preserves line breaks and indentation. It is presentation
data only: never execute a preview or substitute it for the runner's original input.
Redaction is not a general secret scanner; command and edit text are intentionally
visible to the user and their authenticated paired clients.

Claude tool blocks and approval requests, Codex items, ACP tool calls, and declarative
manifest activity all use this boundary. A tool's complete arguments may follow its
initial `started` fact. The ledger must keep this update even if its phase did not
change, while deduplicating identical updates. Completion must preserve known input.
The restore and live-follow paths serialize the same activity facts to local and
paired clients.

Adapter verification must cover command/file details, permission details, a start
with initially empty input followed by complete input, completion, redaction,
Unicode and oversized inputs, and restore/replay. The contract tests live in
`gent-types`, `gent-drivers`, `gent-adapters`, `gent-store`, and the daemon's
permission/ACP integration tests.
