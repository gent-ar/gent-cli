# Sandbox: where it stands and how to resume

## Product decision (2026-11-12)

- Sandbox is OFF by default for every new conversation.
- It is opt-in per conversation from the app's Options modal, under Configuration (below Info).
- The option is enabled only when gentd advertises `agent-chat-sandbox-v1`. Otherwise the switch is disabled and says "Not available on this device." No alert or banner in the chat.
- Releases must not depend on the sandbox working. Unsandboxed behaviour is the shipping path.

## What exists

- Containment is a per-conversation config value (`unsandboxed` default, `sandboxed`). `gent chat create --sandboxed` or an `agent-chat-conversation-config-v1` save sets it. A change saved during a turn is accepted and applies from the next turn.
- One launcher rule: `Containment::requires_sandbox`. A failed admission fails the turn with `failureCode` `sandboxUnavailable` (no helper, pins or package) or `sandboxAdmissionFailed`. It never retries unsandboxed.
- Availability: `permission_backend_registry::sandbox_available()` in `crates/gentd/src/permission_backend_registry.rs` is true only when the launch-time `native(provider)` check passes for Claude, Codex and Claurst. `standalone_capability_profile` advertises `agent-chat-sandbox-v1` from that same predicate, so launch and advertisement cannot disagree.
- App: the switch lives in `app/lib/widget/agent_chat/show_modals/overflow_menu.dart`; availability reaches the UI through the restore snapshot (`GentdConversationRestore.sandboxAvailable`) so remote viewers see the owner's value.
- Ask and Plan are read-only through one shared Gent rule independent of the sandbox (`permission_preflight::read_only_denial`).

## Why Windows is blocked

- `crates/gentd/src/windows_sandbox_runtime.rs` lists no reviewed helper; no `runtime/windows/provider-helper.exe` is staged; no `GENT_WINDOWS_SANDBOX_HELPER_SHA256` pin is compiled in.
- `docs/sandboxing.md` records Windows production as denied: the helper is unsigned and Defender flagged it.
- Release path requires a reviewed descriptor, the exact helper and resources, shell closures and a genuinely signed proxy MSIX (`docs/windows-sandbox-release-build.md`). No production signing identity exists.
- Windows needs two PSEC environments per run (provider and commands); the probes behind that are in `docs/sandboxing.md`.
- On this Windows dev machine every `--sandboxed` turn fails cleanly with `sandboxUnavailable` and writes nothing. macOS and Linux also report the capability absent because the existing launch check always fails there.

## Not tested, because it cannot run here

- Modes (ask, plan, agent) and permission modes (ask-every-time, auto-accept-edits, autonomous, bypass) under a real sandbox.
- A write outside the workspace being blocked.
- Mid-turn sandbox toggle live (code-reviewed only: config save has no active-turn rejection).
- Positive gentd availability test: only the catalog test covers the "available" case.

## Known gaps

- Creating or switching to a sandboxed conversation on a host without the capability is not rejected up front. The save-config protocol returns plain string errors with no typed code, so it would need a new typed rejection frame.
- Remote viewers learn availability only from a snapshot, so a change on the owner's gentd arrives on the next snapshot or reconnect.

## To resume

1. Obtain a signed, reviewed Windows helper and proxy MSIX; follow `docs/windows-sandbox-release-build.md`, then confirm `gent status` lists `agent-chat-sandbox-v1`.
2. Run the live matrix with `gent chat create --sandboxed` on all three providers: each mode, each permission mode, outside-workspace write, mid-turn toggle, ask-mode refusal.
3. Decide whether to add a typed up-front `sandboxUnavailable` rejection to the config-save path.
4. Only then consider enabling the option by default anywhere; the current decision is opt-in.
