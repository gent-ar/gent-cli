# Malformed provider-frame evidence contract

`malformed_tolerance` has two deliberately separate proof obligations.

1. Deterministic driver tests prove the parser/framer boundary: invalid JSON,
   incomplete documented fields, unknown frame kinds, oversized NDJSON, and a
   valid frame after rejection never crash, retain an invalid partial frame, or
   mutate a terminal session.
2. A future recorded matrix cell proves a provider actually emitted a relevant
   abnormal frame. It must not turn a local injector, proxy, replay, or parser
   fixture into a claim about a vendor.

The installed Claude and Codex CLIs currently document no bounded output-fault
control. Do not try to make a provider produce corrupt stdout, alter its stream,
or send malformed protocol input: that would either test Gent's own injection
or risk an unaudited provider action. Both manifest cells therefore remain
`capture_required`.

## Future live capture prerequisite

Only record the cell when the provider publishes a documented test/diagnostic
control that emits a malformed or unknown output frame during an attended,
ephemeral, read-only, tool-free run. Retain no raw output. Instead retain the
redacted structural input shape, a SHA-256 digest of the reviewed shape, the
vendor documentation URL/control, the exact normalizer diagnostic, and one
ordinary frame observed after the fault.

Validate a reviewed candidate before changing the manifest:

```sh
python3 tools/validate-malformed-driver-evidence.py candidate.jsonl
```

The validator never starts a provider, writes a fixture, or accepts an injected
source. Its `--describe` mode gives automation the exact future requirement:

```sh
python3 tools/validate-malformed-driver-evidence.py --describe
```

It validates the *additional* malformed-evidence facts only. The normal
public-driver manifest validator still enforces provenance, redaction, matrix
identity, and the strict `--require-live` gate. A candidate is not evidence
until it passes both validators and a human review.
