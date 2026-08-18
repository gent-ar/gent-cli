#!/usr/bin/env python3
"""Provider-neutral semantic checks for the offline driver transcript corpus."""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Any


PROVIDERS = {"claude", "codex", "claurst"}
MODES = {"ask", "plan", "agent"}
EFFORTS = {"low", "medium", "high"}
DIGEST = re.compile(r"sha256:[0-9a-f]{64}$")


@dataclass
class Run:
    history_ordinal: int
    latest_ordinal: int


def validate_semantics(
    provider: str, events: list[dict[str, Any]], attachments: list[dict[str, Any]] | None, location: str
) -> None:
    """Ensure sanitized records describe coherent Gent-owned conversation state."""
    state = _State(provider, attachments or [], location)
    for event in events:
        state.apply(event["type"], event["data"], event["sequence"])
    state.finish()


class _State:
    def __init__(self, provider: str, manifest_attachments: list[dict[str, Any]], location: str) -> None:
        self.provider = provider
        self.location = location
        self.conversation_seen = False
        self.active_run: str | None = None
        self.runs: dict[str, Run] = {}
        self.turns: set[str] = set()
        self.plans: dict[str, tuple[int, str]] = {}
        self.goals: dict[str, int] = {}
        self.attachments: set[tuple[str, str, int]] = set()
        self.manifest_attachments = {
            (item["contentDigest"], item["mediaType"], item["byteLength"])
            for item in manifest_attachments
        }
        if len(self.manifest_attachments) != len(manifest_attachments):
            self.fail("manifest attachment metadata must be unique")

    def apply(self, kind: str, data: dict[str, Any], sequence: int) -> None:
        if kind == "conversation":
            self.conversation(data)
        elif kind == "run":
            self.run(data)
        elif kind == "turn":
            self.turn(data)
        elif kind == "message":
            self.message(data)
        elif kind == "activity":
            self.activity(data)
        elif kind == "plan":
            self.plan(data)
        elif kind == "attachment":
            self.attachment(data)
        elif kind == "terminal":
            self.terminal(data, sequence)

    def finish(self) -> None:
        if self.attachments and not self.turns:
            self.fail("attachment event requires a known turn")

    def conversation(self, data: dict[str, Any]) -> None:
        if self.conversation_seen:
            self.fail("scenario may contain only one conversation event")
        self.required(data, "conversationId", "provider", "model", "effort", "mode")
        if data["provider"] != self.provider or data["provider"] not in PROVIDERS:
            self.fail("conversation provider must match the scenario provider")
        if data["effort"] not in EFFORTS or data["mode"] not in MODES:
            self.fail("conversation selection is invalid")
        self.conversation_seen = True

    def run(self, data: dict[str, Any]) -> None:
        self.required(data, "runId", "contextPolicy", "historyOrdinal")
        run_id = self.identifier(data["runId"], "runId")
        history = self.natural(data["historyOrdinal"], "historyOrdinal", allow_zero=True)
        if run_id in self.runs or data["contextPolicy"] not in {"preserve", "clear"}:
            self.fail("run identity or contextPolicy is invalid")
        parent = data.get("parentRunId")
        if parent is not None:
            parent = self.identifier(parent, "parentRunId")
            parent_run = self.runs.get(parent)
            if parent_run is None:
                self.fail("parentRunId must reference an earlier run")
            if data["contextPolicy"] == "preserve" and history != parent_run.latest_ordinal:
                self.fail("preserved context must use the parent's frozen history ordinal")
        if data["contextPolicy"] == "clear" and history != 0:
            self.fail("clear context must use historyOrdinal zero")
        for key, values in (("provider", PROVIDERS), ("effort", EFFORTS), ("mode", MODES)):
            if key in data and data[key] not in values:
                self.fail(f"run {key} is invalid")
        self.runs[run_id] = Run(history, history)
        self.active_run = run_id

    def turn(self, data: dict[str, Any]) -> None:
        self.required(data, "turnId", "ordinal")
        turn_id = self.identifier(data["turnId"], "turnId")
        ordinal = self.natural(data["ordinal"], "ordinal", allow_zero=False)
        run_id = data.get("runId", self.active_run)
        if turn_id in self.turns or not isinstance(run_id, str) or run_id not in self.runs:
            self.fail("turn must belong to the active or named known run")
        run = self.runs[run_id]
        if ordinal <= run.latest_ordinal:
            self.fail("turn ordinal must advance its run history")
        run.latest_ordinal = ordinal
        self.turns.add(turn_id)

    def message(self, data: dict[str, Any]) -> None:
        self.required(data, "role", "text")
        if data["role"] not in {"user", "assistant", "system"}:
            self.fail("message role is invalid")

    def activity(self, data: dict[str, Any]) -> None:
        self.required(data, "kind", "status")
        if data["kind"] != "goal":
            return
        self.required(data, "goalId", "revision", "summary")
        goal_id = self.identifier(data["goalId"], "goalId")
        revision = self.natural(data["revision"], "goal revision", allow_zero=False)
        previous = self.goals.get(goal_id, 0)
        if revision <= previous:
            self.fail("goal revision must advance")
        self.goals[goal_id] = revision

    def plan(self, data: dict[str, Any]) -> None:
        self.required(data, "planId", "revision", "digest", "status")
        plan_id = self.identifier(data["planId"], "planId")
        revision = self.natural(data["revision"], "plan revision", allow_zero=False)
        digest = data["digest"]
        if not isinstance(digest, str) or not DIGEST.fullmatch(digest):
            self.fail("plan digest must be a SHA-256 identity")
        previous = self.plans.get(plan_id)
        if previous is not None and (
            revision < previous[0] or (revision == previous[0] and digest != previous[1])
        ):
            self.fail("plan revision and digest must remain exact")
        self.plans[plan_id] = (revision, digest)

    def attachment(self, data: dict[str, Any]) -> None:
        self.required(data, "contentDigest", "mediaType", "byteLength", "turnId")
        digest = data["contentDigest"]
        media_type = data["mediaType"]
        length = data["byteLength"]
        if not isinstance(digest, str) or not DIGEST.fullmatch(digest):
            self.fail("attachment digest must be a SHA-256 identity")
        if not isinstance(media_type, str) or not media_type.strip():
            self.fail("attachment mediaType is invalid")
        if not isinstance(length, int) or length < 0:
            self.fail("attachment byteLength is invalid")
        if self.identifier(data["turnId"], "turnId") not in self.turns:
            self.fail("attachment must reference an earlier turn")
        identity = (digest, media_type, length)
        if identity not in self.manifest_attachments:
            self.fail("attachment must be declared in the manifest")
        if identity in self.attachments:
            self.fail("attachment metadata may appear only once per scenario")
        self.attachments.add(identity)

    def terminal(self, data: dict[str, Any], sequence: int) -> None:
        self.required(data, "outcome")
        if "cursor" in data:
            cursor = self.natural(data["cursor"], "terminal cursor", allow_zero=False)
            if cursor != sequence:
                self.fail("terminal cursor must equal its durable event sequence")

    def required(self, data: dict[str, Any], *keys: str) -> None:
        for key in keys:
            if key not in data:
                self.fail(f"{key} is required")
            if key not in {"revision", "historyOrdinal", "ordinal", "byteLength", "cursor"}:
                if not isinstance(data[key], str) or not data[key].strip():
                    self.fail(f"{key} must be a non-empty string")

    def identifier(self, value: Any, field: str) -> str:
        if not isinstance(value, str) or not value.strip():
            self.fail(f"{field} must be a non-empty string")
        return value

    def natural(self, value: Any, field: str, *, allow_zero: bool) -> int:
        if not isinstance(value, int) or isinstance(value, bool) or value < (0 if allow_zero else 1):
            self.fail(f"{field} is invalid")
        return value

    def fail(self, reason: str) -> None:
        raise ValueError(f"{self.location}: {reason}")
