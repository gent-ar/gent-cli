# Conversation archive

Clients negotiate `agent-chat-archive-v1` and send `archiveConversation` with `requestId`,
`receiptId` and `conversationId`. Gentd replies `conversationArchived` with `requestId`, the
settled `receipt` and `conversationId`. Retrying with the same receipt returns the original
receipt; a new receipt for an archived conversation also succeeds and changes nothing. An unknown
conversation is rejected with `conversationNotFound`; a client that did not negotiate the
capability gets `unsupportedCapability`.

Archiving appends one row to `archived_conversations` (schema v30) in the same transaction as the
receipt. Nothing is deleted: the ledger stays append-only and every committed cursor stays valid.
Conversation lists (`gent conversation list`) leave archived conversations out. Summary, detail,
transcript, activity and projection follow reads keep working. A new prompt is rejected with
`conversationArchived`.

`gent chat archive --conversation-id <id>` sends the intent.

# Selection summary deltas

A selection switch and a conversation configuration save, from any client, append a `summary`
projection event in the same transaction. `followConversation` turns it into a top-level delta
`{"type":"summary","body":{"cursor":{"value":N},"runId":"<current run>","summary":{...}}}` whose
summary is the conversation summary a detail read returns. A follow after that cursor never
replays it, so a client restored from a snapshot and following from its cursor applies it once.
