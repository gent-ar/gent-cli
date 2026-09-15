# Public driver transcript capture

The helper prints exact provider-specific capture commands for each unrecorded
cell, including model, transport, and canonical output path. Add `--run --confirm`
only in an attended, reviewed capture session.

Use `--require-live` only at the real-provider evidence gate. It deliberately
fails until every cell is a redacted live recording or a reasoned recorded
absence; synthetic fixtures never satisfy that gate. A claimed live capture
also requires canonical executable identity and SHA-256, provider transport,
platform, RFC3339 capture time, run identifier, and attestation digest. The
capture helper's digest is a reproducible hash of reviewed redacted metadata
and normalized frames; raw provider output is bounded, discarded, and
deliberately not claimed as attested. These are structural provenance checks,
not a substitute for the planned signed real-provider artifact and
normalized-event replay gate.

Refresh an approved safe Claude/Codex cell with the redaction-first helper:

```sh
python3 tools/capture-public-driver-transcript.py claude full_turn \
  --model haiku --output fixtures/public-driver-transcripts/claude-full-turn.jsonl \
  --confirm-live-capture --update-manifest
```

It retains raw output only in memory, writes normalized facts, and refuses to
run without explicit confirmation. The native Claude subagent row uses the
separate reviewed `tools/capture-claude-subagent-transcript.py` helper: it
allows only `Task(gent_probe)` and records a correlated native `Agent` call,
matching tool result, and successful terminal event—not prompt text. Other
matrix rows need scenario-specific reviewed captures. Claurst local-runtime
validation is separate from this Claude/Codex capture corpus. It has no hosted
credentials or endpoint, and this section does not claim a live local-model
generation.

An observed absence can be kept as diagnostic context, but never satisfies the
real-provider `--require-live` gate: authority requires positive, redacted,
scenario-specific live capture. A parser error before a provider turn, help
output, or an unavailable flag is not provider evidence.

Codex approval, persistent-permission, plan, compaction, MCP, interrupt, and
steering scenarios use the documented app-server JSON-RPC harness rather than
one-shot `codex exec`; it has a provider-free dry run and never changes the
matrix automatically. It emits a candidate fixture only after the scenario's
correlated native protocol conditions are observed. Its MCP helper requires an
already-authenticated, isolated `CODEX_HOME` and never copies or reads credentials:

```sh
python3 tools/capture-codex-app-server-transcript.py plan_mode \
  --model gpt-5.6-luna \
  --output fixtures/public-driver-transcripts/codex-plan-mode.jsonl --dry-run
```
