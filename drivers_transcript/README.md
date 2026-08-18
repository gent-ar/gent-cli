# Driver transcript corpus

This committed directory is Gent’s reusable, offline development corpus for
driver tests, coverage, parser fixes, and lifecycle regression tests. It is
off by default: ordinary Gent sessions must not write a capture. A developer
explicitly starts a recording only for a real session or a test scenario, then
sanitizes and reviews it before committing it here.

Use it for both synthetic test conversations and explicitly captured real user
conversations. Before spending tokens on another Claude, Codex, or private
Claurst test, look here and replay an existing scenario whenever it covers the
behavior being changed.

When recording is explicitly enabled, capture the whole typed conversation
boundary for behavior that is not already represented:

- Conversation, run, turn, model, effort, mode, and durable cursor metadata.
- User and final assistant text needed to reproduce the behavior.
- Normalized tool, task, subagent, permission, interrupt, compaction, and
  terminal lifecycle facts.
- Reviewed plans, approval/rejection outcomes, context policy, and receipts.
- Attachment and image/file metadata, turn associations, size, media type, and
  SHA-256 storage identity.

Use one scenario directory per provider and behavior, with a concise
`manifest.json` and bounded `events.jsonl`. Keep records typed and ordered;
tests must replay the normalized records rather than calling a provider.
Run `python3 tools/validate-driver-transcript-corpus.py` before committing;
it rejects raw-session fields, known credential markers, source paths, unsafe
files, and malformed scenario records. This automated screen supplements,
rather than replaces, human review of real conversation text.

Never commit credentials, API keys, tokens, cookies, account details, private
endpoints or routing, provider-native session/resume IDs, raw process frames,
environment variables, hidden reasoning, unrestricted tool output, source
paths, or attachment bytes. Attachments are represented by their existing
content-addressed metadata only. Claurst records remain normalized and
credential-free; bridge-specific raw material stays app-private.

This corpus is distinct from `fixtures/public-driver-transcripts/`: evidence
fixtures prove narrowly redacted compatibility claims, while this corpus is the
broader sanitized regression dataset. Neither corpus authorizes a provider,
changes observer mode, injects a plan into a provider, or enables recording
for a later normal session.
