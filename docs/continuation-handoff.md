# Gent CLI continuation handoff

Use this document with the implementation plan and repository state when
resuming work in a new conversation. Facts are current through 2026-08-19; this
does not claim observer-mode `gentd` has live provider authority.

## Repository and safety

- Repository: `/Users/ivanmatiasfort/Clouseau/gent-cli`, branch `main`, remote
  `git@github.com:gent-ar/gent-cli.git`.
- Inspect the Gent worktree before editing. Preserve any staged user work; do
  not reset or checkout. Commits and pushes require the user's explicit approval.
- Do not modify `/Users/ivanmatiasfort/Clouseau/clouseau-app` during standalone
  Gent work. Its unrelated dirty files include Mermaid assets, Mux provider,
  agent-chat widgets/tests, and Excalidraw Mermaid.
- Use `apply_patch` for edits. Every hand-authored source/config/document/script
  is at most 300 lines; run `python3 tools/check-architecture.py`.
- For the unchanged 90% coverage gate on a space-constrained machine, use the
  isolated target workflow in [coverage workflow](coverage-workflow.md); never
  delete normal `target/` artifacts merely to make an instrumented build fit.

## Non-negotiable architecture

- `gentd` is the only composition root and future ledger writer.
- `gent` is protocol-only and depends only on `gent-protocol` and `gent-types`.
  It must not import the store, drivers, network listeners, or provider spawners.
- `gent-core` stays pure: no database, process, HTTP, or CLI imports. Domains
  use typed ports/reducers; composition roots wire them.
- Public Gent never contains Claurst credentials, endpoints, or routing. The
  private bridge implementation belongs in app-owned private code behind a port.
- Default `gentd` is hard observer: it must not start providers, MCP, Git
  mutation, automations, watchers, schedulers, or a private bridge.

## Product contract: one shared harness

`gent` terminal and the closed native app are equal first-class clients of the
same daemon-owned Gent harness. Both must support conversations, sessions,
browsing, create/prompt/follow-up, provider/model/effort/mode, live response
and tool state, plans, clear context, permissions, login, reconnect, and cursor
resume. The native app adds only private product features: local/LAN/relay, IDE,
system UI, voice, pairing, and app automations.

Neither client parses provider stdout, owns a lifecycle reducer, starts a
provider, or writes the Gent ledger. Providers provide raw facts; `gentd`
normalizes, persists, cursor-orders, and streams the client-visible truth.

## Current implemented batch

- Default signed automatic update machinery, Windows installer/runtime bootstrap
  support, CI gates, and release documentation.
- Durable permission modes: Default, Plan, Auto-Accept Edits, Autonomous, and
  one-time-consented persistent Bypass. Plan remains Plan after approvals;
  broad modes fail closed without verified sandbox containment.
- Secret-free provider-auth discovery/login contract, pure reducer, `gent auth
  status|login`, daemon adapter boundary, and observer-safe refusal. No live
  provider login is composed.
- Reviewed-plan authority foundation: immutable trusted artifacts, pure reducer,
  strict protocol frames, a `ReviewedPlanLedger`, fresh-schema storage, and atomic child
  reservation. Approval rechecks exact plan/digest, parent, epoch, policy,
  receipt/idempotency; clear context records ordinal zero and no native session.
  Observer `gentd` still does not advertise or compose this authority.
- Durable, provider-neutral conversation goals: a fresh-schema `GoalLedger`,
  pure revision/epoch reducer, capability-gated IPC, and `gent goal
  create|read|list|transition`. Positional `/goal <summary>` requires an exact
  existing conversation/run binding and cannot reach a provider in observer mode.
- The dormant approved public-driver seam can inject a fresh active-goal resolver
  per Claude/Codex turn; terminal, stale, malformed, ambiguous, or mismatched
  goals are omitted/rejected before a runner. Bootstrap still injects no resolver.
- A committed prompt response now carries the ledger-assigned conversation, run,
  and turn identities. `gent <prompt>` defaults its new selection to Ask and
  returns those identities without guessing a later lifecycle or keeping a
  terminal-owned correlation map.
- A sealed dormant ordinary authority accepts Claude/Codex grants only from one
  signed release artifact after nested evidence and compatibility verification.
  It derives its owner/epoch from active daemon state and accepts only Claude/Codex
  Ask or Plan selections. Its shared lifecycle router resolves durable provider
  selection and arms only the matching bounded host; a paired Notify cadence
  replays ledger recovery then polls only active work. Bootstrap constructs neither.
- A private provision settlement transaction records only a verified install and its
  package-policy/Node/receipt provenance with its terminal receipt. Ambiguous effects
  become unprovable; the dormant Claude/Codex resolvers read and recheck this lock
  directly, with no prefix or `PATH` discovery.
- Dependency receipt reservation is now an effect-free shared runtime rule. The
  obsolete generic provisioning owner was removed; only the prompt-scoped path
  may later settle denial/mismatch without npm, atomically write a verified lock,
  and turn any recovered accepted receipt unprovable without replay. Observer
  bootstrap and capabilities still construct neither an installer nor authority.
- Post-install provider locks begin explicitly unbound. A narrow compatibility
  port revalidates the signed manifest at the provisioning operation's current
  time, binds only its exact provider/version/digest entry, and otherwise makes
  the effect unprovable with no runnable lock. This avoids treating daemon-start
  time or a caller-supplied compatibility label as authority.
- The dormant Claude/Codex composition uses a fresh daemon clock to
  reauthorize a durable lock immediately before each provider effect, including
  session resume. It refuses a now-expired or revoked entry before launch and
  remains absent from observer composition; no snapshot or second state store
  is introduced.
- A distinct, read-only private readiness decision checks only durable
  Claude/Codex locks. Missing or changed locks produce a Gent-generated install
  review; unreadable provenance fails closed, and Claurst never enters this npm
  path. It has no IPC frame, bootstrap composition, prompt hook, or installer.
- The prompt ledger exposes a private exact-run admission method. It confirms
  the expected reviewed run inside the prompt write transaction and rejects a
  concurrent selection change without saving a message. New send prompts are
  held as `awaiting_readiness`, never claimable by a lifecycle runner, until an
  internal epoch/current-run-fenced authority releases that exact prompt.
- The client-visible delivery value is `awaitingReadiness`. Generic chat
  persistence does not wake an ordinary lifecycle for a held prompt; the future
  readiness authority must release the exact durable prompt before it wakes it.
- provider-readiness-v2 is a separately negotiated, exact conversation/run
  read surface in an explicit private-authority profile only. Its public review
  digest binds the provider, action, instruction, consent requirement, package
  name/version/integrity, and signed-policy digest. A profile cannot advertise
  readiness without that exact-review authority; observer and chat-only profiles
  preserve capability absence. It does not authorize a provider launch.
- prompt-provider-provision-v1 has a strict, uncomposed confirmation DTO. A client may echo
  only its provision receipt, exact held prompt/conversation/run, consent, epoch, and the
  daemon-issued review digest; provider/action/package/path/plan/policy are absent. A new
  SQLite port atomically claims the accepted receipt and changes that exact `awaiting_readiness`
  send to `provisioning`, which blocks a competing selection switch, then atomically persists a
  verified public-provider lock,
  terminal provision receipt, and its release. Its private command additionally binds the
  daemon-selected package name/version/integrity/policy digest; Gent rechecks all coordinates
  immediately before npm and requires the persisted installation provenance to match. Ambiguous/
  recovered effects settle that exact reservation `unprovable` without release or replay.
  Consent refusal and stale review digest settle rejected, retryable no-effect receipts. A
  daemon-only boundary derives current selection, policy package, and review before atomically
  admitting the effect; its strict capability-gated IPC transport validates every correlation.
  Only an explicit injected private-authority `RuntimeFacade` constructor exposes it. Shipped
  bootstrap remains observer-only and cannot advertise the capability.
- Agent-chat detail now includes the durable `currentRunId`, calculated by the
  same selected-run ordering as prompt ownership. Clients must carry that
  identity into future readiness and prompt-fence requests rather than infer it.
- The obsolete ordinary-authority path/key bootstrap parser was removed. A future
  explicit authority bootstrap must verify one signed release artifact against the
  locked Node runtime and compose only its in-memory grants; it accepts no client
  evidence, provider keys, coordinator, or epoch.
- Private ordinary lifecycle hosts reject shutdown before recovery and let
  recovered idle hosts drain without manufacturing a prompt wake. The router
  aggregates explicit shutdown/escalation/completion; Unix IPC has a transient,
  cancellation-aware listener/connection drain seam with no task abort or
  durable lifecycle side effect. Neither is composed by the observer daemon.
- The ordinary cadence now starts closed, opens only after ledger recovery
  reaches idle, and refuses an earlier shutdown without a recovery wake. Its
  sealed facade ingress holds a transient admission permit before prompt
  persistence through the post-commit wake; shutdown closes admission then
  waits for existing permits before draining. This is process-local control,
  not a snapshot, ledger record, or observer capability.
- Committed, redacted development driver corpus plus public normalized live
  full-turn captures for Codex, Claude Haiku, and Claude Sonnet. Capture stays
  opt-in; corpus records are not lifecycle authority or evidence-gate substitutes.
- Clear context creates a child boundary with history ordinal zero; it does not
  delete history or reuse a provider session. Provider switches create child runs.
- Public driver/process/backpressure/binary-lock/session-normalization seams are
  implemented but dormant. They are not live daemon authority.
- Reviewed-plan, Flutter handoff, realtime client lifecycle, and terminal/native
  parity documents are added and linked from implementation status.
- A Gent-native multi-agent orchestration contract is planned: typed task graphs,
  isolated worktree leases, `/fanout`, `/cross-review`, cross-vendor findings,
  cursor-resumable state, and custom harness profiles. It is not composed or
  advertised; see `docs/multi-agent-orchestration-plan.md`.

## Required realtime experience

1. One private `.gentd` profile owns one demand-started, multiplexed `gentd` and
   one ledger. The app starts/locates it once and uses long-lived local IPC;
   `gent` terminal uses the same protocol. It is not a permanent background
   service and it is never one `gent` or provider process per prompt/conversation.
   The daemon stays up for active work, pending decisions, or connected clients,
   then may obey an explicit idle policy.
2. Clients negotiate, read conversation index/content pages, and attach
   cursor-resumable event/activity subscriptions for the selected run.
3. Create/prompt/follow-up/selection/plan/permission/login are typed commands
   with receipts, idempotency, epoch fences, and terminal outcomes.
4. Daemon checks authority, policy, sandbox, binary lock, and evidence before
   spawn/resume. Provider facts persist before publication; failures are durable.
   It alone applies shared resource budgets, queues excess durable prompts, and
   drains owned provider process trees. A conversation/run has a canonical
   ledger workspace binding; daemon or client cwd is never authoritative.
5. On disconnect or epoch change, clients re-read bounded durable pages and resume
   from their last acknowledged cursor: no duplicate output or invented loading state.
   Snapshot state, recovery caches, mirrored state, and replacement layers are
   prohibited. Derived views are disposable/non-authoritative: never serialize,
   transmit, or recover from them. Clients and daemon reread bounded immutable
   pages (from cursor zero when needed), then replay normalized facts after an
   accepted cursor.

## Next implementation order

1. **Done (`83d3495`):** bind the signed release's artifact digest through
   `ProviderPromptProvisionCommandBinding`/`ProviderInstallProvenance`, add a receipt fingerprint,
   and re-verify the same signed release immediately before npm. See "Latest continuation state"
   below for the exact files. `daemon_bootstrap.rs` has a zero-line diff throughout.
2. **Done, uncommitted (2026-08-19):** prove that profile end-to-end — persist-before-broadcast,
   cursor reread/reconnect, exact-retry idempotency, and terminal settlement (consent refusal,
   unprovable) — via `crates/gentd/src/prompt_provider_provision_profile_tests.rs`, a real
   `RuntimeFacade` driven over the real wire codec. See "Latest continuation state" below.
   Backpressure/process-tree drain, turn follow, provider/context switch, and `/goal` projection
   belong to the Claude/Codex session-runner authority (item 4), not this npm-install profile —
   deferred there, not skipped.
3. Add reviewed-plan authority composition only after the lifecycle and evidence
   gates; clients never inject provider plans and observer remains absent.
4. Compose task-graph scheduling only after public-driver authority: each node
   gets a leased isolated worktree, fresh goal projection, durable settle, and
   an independently locked profile. Claurst stays a private bridge port.
5. Compose terminal browser parity over the same frames; terminal UI has no
   independent plan, permission, or lifecycle logic.
6. Provider-auth authority: typed `askTool`, locks, sandboxed edge, live proof.
   Login route selection is public; credential values never cross public IPC.
7. Private app-owned Claurst bridge plus private CI evidence; then live MCP/Git
   behind ports and receipts. Gent-canvas/forge are later; pairing/app
   automations stay app-only.
8. Capture strict real evidence, then seek explicit release authorization and
   only later begin separately authorized Flutter wiring.

## Evidence status

- Two strict Codex cells are recorded. Four strict cells are missing: Claude persistent
  permission, compaction, malformed tolerance, and Codex malformed tolerance.
  Claurst needs private bridge/CI evidence. Never fabricate recordings.
- `--permission-prompt-tool` is undocumented (absent from `claude --help` on 2.1.235) but real and
  functional, verified live — matches the native app's own Claude adapter, which uses it
  unconditionally via `control_request`/`control_response` over `stream-json` stdio, never an
  external MCP server. `tools/capture-claude-persistent-permission-transcript.py` now uses that
  exact pattern; persistent-permission capture is unblocked. Compaction/malformed-tolerance capture
  (both vendors, `0.144.1` observed for Codex) still has no documented bounded signal to capture.
- Use `python3 tools/update-public-driver-transcripts.py`; keep captures redacted
  and admitted only through the transcript manifest.
- The `drivers_transcript/` corpus is a committed, sanitized development asset.
  Normal Gent sessions never write it; validate it with
  `python3 tools/validate-driver-transcript-corpus.py`.
- Do not claim a release for this uncommitted batch. Windows scheduled-task
  execution needs Windows CI and was not run locally on macOS.

## Verification passed after this batch

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
python3 tools/test-gent-auto-update.py
python3 tools/test-installer-runtime-bootstrap.py
python3 tools/check-architecture.py
git diff --check
git diff --cached --check
```

The full workspace suite passed, including CLI reviewed-plan socket tests and
the daemon facade observer test. Re-run all of it after nontrivial changes.

## Primary documents

- `README.md`: repository contract and user-facing milestone.
- `docs/implementation-status.md`: implemented inventory and live-backend gates.
- `docs/realtime-agent-chat-client-plan.md`: explicit realtime and parity flow.
- `docs/flutter-handoff-v1.md`: native IPC boundary.
- `docs/agent-chat-execution-plan.md`: review/start/clear-context contract.
- `docs/provider-auth-plan.md`: login contract and authority gate.
- `docs/architecture.md`: crate dependency/composition law.
- `docs/multi-agent-orchestration-plan.md`: planned daemon-owned fanout and
  cross-vendor review contract.

Before major scope decisions, read the original app planning source only:
`/Users/ivanmatiasfort/Clouseau/clouseau-app/GENT-CLI/README.md`, then
`00-PLATFORM-CONTRACT.md` through `10-LIVE-LIFECYCLE-AND-SELF-UPDATE.md`.

## Latest continuation state (2026-08-19, revised)

- `main` is at `83d3495` (item 1, committed). Items 2 and 3 below are **uncommitted**,
  pending approval. `clouseau-app` is read for reference only, never edited.
- Item 1 (`83d3495`): bound the signed release's digest through
  `ProviderPromptProvisionCommandBinding`/`ProviderInstallProvenance`, added
  `Command::receipt_fingerprint_sha256()`, bumped schema to `v10`, and made the effect
  re-verify the release immediately before npm (mismatch settles `Unprovable`, never
  retried). Split five oversized files into siblings.
- Item 2: `prompt_provider_provision_profile_{support,tests}.rs` compose a REAL
  `RuntimeFacade` (`from_state_with_prompt_provider_provision_authority`, still uncalled by
  bootstrap) — real ledger, boundary, digest-bound provisioner, signed-release fixture; only
  the npm/binary boundary is a double. Four tests drive it over the real wire codec:
  persist-before-broadcast, cursor reread/reconnect, exact-retry idempotency, terminal
  settlement. Turn-follow/backpressure/context-switch/`/goal` belong to item 4, not this
  npm-install profile.
- Item 3 (evidence tooling, not authority code): "Claude Code lacks `--permission-prompt-tool`"
  was wrong — verified live, undocumented but functional; the app uses it unconditionally via
  `control_request`/`control_response` over `stream-json` stdio, never an external MCP server.
  Rewrote the capture script + added `tools/public_driver_capture_permission.py` on that proven
  pattern; persistent-permission capture is unblocked. Corrected the same stale claim in
  `docs/implementation-status.md`.
- Verification passed for items 1–2; item 3 is Python-only (syntax + its own no-provider tests
  passed; no live capture run yet — needs your go-ahead). Standing decisions: reuse the Ed25519
  trust root; no `Sha256Digest` newtype; never embed/depend on the app's code at gent-cli
  runtime, but DO mine its proven protocol knowledge into gent-cli's own Rust implementation;
  reject independent evidence/key paths, snapshots, fake containment, public Claurst.

Resume here: capture the now-unblocked `claude/permission_persistent` cell (needs your authenticated
Claude CLI + go-ahead). Compaction/malformed-tolerance (both vendors) still need a documented
bounded signal. Item 4 (Claude/Codex authority composition) is the real unlock for items 3+ —
build it by porting proven behavior from `{claude,codex}_driver.dart`, not from scratch. Terminal
parity (item 5) is independently tractable meanwhile.
