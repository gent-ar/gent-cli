# Native-app Gentd integration

Gentd is the shared standalone authority for terminal and native app. The app
is presentation plus IPC: it has no provider subprocess, provider stdout
parser, provider-native session store, or Gent ledger writer on the Gentd
route.

## Current integration

- New Gentd conversations use the durable daemon conversation ID as their app
  conversation identity and the returned run ID as their selected run.
- Sends stage attachments with `attachments-v1`, persist an optimistic app
  bubble, and submit text plus durable attachment IDs through
  `agent-chat-intents-v1`.
- Transcript restoration uses bounded daemon pages. Live output uses exact-turn
  follow and activity/permission reads instead of provider text inference.
- Model, provider, effort, and mode changes use immutable run selection.
- The app produces one MCP config file from its connector state and gives that
  file to the packaged Gentd process. The app does not configure individual
  providers.
- Claurst is selected like Claude or Codex. Its model catalogue and download
  progress come from Gentd; execution is local through Claurst and llama.cpp.

## Remaining release limits

1. The native projection must retain and filter local-model `requestId` values
   when following download events for a selected prompt.
2. Claude child-activity sidecar correlation needs a shared child-scoped
   transcript contract before it can be exposed through Gentd.
3. Live evidence still needs the documented Claude and Codex malformed and
   compaction cases. Claurst has completed a clean packaged local-runtime
   macOS turn; Windows and Linux release-hardware runs remain.
4. The app must retain no direct-provider fallback for a failed Gentd action.
   It shows the authoritative Gentd error or reconnect state.

## Cutover rule

Remove direct app provider execution as each equivalent Gentd capability is
verified. Do not add a second route or fallback driver. Conversations and sessions are Gentd-owned from creation and
remain shared between terminal and app.
