# Malformed provider-frame tolerance

Malformed-frame tolerance is a property of Gent's own parsers, not a fact about
a provider, so it is proven deterministically in this repository and is not part
of the recorded public-driver matrix or the signed ordinary-authority evidence
set.

## Why it is not a capture cell

A recorded matrix cell claims that a named provider version, on a named
platform, actually emitted the frames in the fixture. Claude Code and Codex
publish no bounded output-fault control, so the only way to produce a malformed
frame from them is to inject, proxy, replay, or shim the stream. Any of those
would turn Gent's own injector into a claim about a vendor, which is
fabrication. `malformed_tolerance` therefore has no cell in
`fixtures/public-driver-transcripts/manifest.yml`, no scenario in
`gent_testkit::REQUIRED_SCENARIOS`, and no proof in
`Claude/CodexAuthorityEvidencePayload::scenarios`.

## Where tolerance is proven instead

Every obligation below is a deterministic Rust test that feeds the fault
straight into the shipped parser and asserts a typed diagnostic plus survival.

| Obligation | Proof |
| --- | --- |
| Unparseable JSON on the Claude stream-json boundary | `crates/gent-drivers/tests/claude_runner.rs` (`malformedClaudeFrame`) |
| Unparseable JSON on the Codex app-server boundary | `crates/gent-drivers/tests/public_protocol_edges_replay.rs` (`malformedCodexFrame`) |
| Incomplete documented fields | `crates/gent-drivers/tests/public_protocol_edges.rs` |
| Unknown frame kinds | `crates/gent-drivers/tests/public_protocol_edges.rs` |
| Oversized NDJSON lines | `crates/gent-drivers/src/ndjson.rs`, `src/output_pump.rs`, `tests/claude_runner.rs`, `tests/codex_runner.rs` (`oversizedProviderFrame`) |
| A valid frame after a rejected one | `crates/gent-drivers/tests/public_protocol_edges_replay.rs`, `tests/claude_runner.rs` |
| A frame after a terminal session never mutates it | `crates/gent-drivers/tests/session_errors.rs` |

`fixtures/public-driver-transcripts/manifest.yml` records the same pointer in
its validated `malformed_tolerance_proof` field, so the corpus cannot silently
drop the statement.

## Reviewing a hypothetical future live candidate

If a vendor ever ships a documented, bounded output-fault control, a candidate
recording must still be reviewed before anybody proposes reinstating a cell.
`tools/validate-malformed-driver-evidence.py` is that review gate: it checks the
provenance, redaction, fault boundary, and post-fault continuation facts a live
candidate would have to declare, and it never starts a provider, writes a
fixture, or accepts an injected source.

```sh
python3 tools/validate-malformed-driver-evidence.py candidate.jsonl
python3 tools/validate-malformed-driver-evidence.py --describe
```

Passing it is a precondition for a human review, not evidence by itself.
