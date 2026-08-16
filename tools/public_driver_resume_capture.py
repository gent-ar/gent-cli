"""Redaction-first Codex resume capture, isolated from normal Codex state."""

from __future__ import annotations

import json
import os
import shutil
import tempfile
from pathlib import Path

from public_driver_capture_stream import capture


SEED_MARKER = "GENT_RESUME_SEED_OK"
RESUME_MARKER = "GENT_RESUME_OK"


def normalized_frames() -> list[dict[str, object]]:
    """Return only the provider-observed relation, with the native identity redacted."""
    return [
        {"in": {"nativeType": "thread.started", "resumeRequested": False, "sessionIdentity": "redacted_seed"}, "expect": "initialized_session", "expectFields": {"sessionPersistence": True}},
        {"in": {"nativeType": "turn.completed", "sessionIdentity": "redacted_seed"}, "expect": "completed_turn", "expectFields": {"terminal": True}},
        {"in": {"nativeType": "thread.started", "resumeRequested": True, "sessionIdentity": "redacted_seed", "sameAsSeed": True}, "expect": "resumed_session", "expectFields": {"providerVerifiedResume": True}},
        {"in": {"nativeType": "turn.completed", "sessionIdentity": "redacted_seed"}, "expect": "completed_turn", "expectFields": {"terminal": True}},
    ]


def _thread_identity(raw: str) -> str:
    """Require one native thread start and a completed turn; never return raw output."""
    identities: set[str] = set()
    completed = False
    for line in raw.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "thread.started":
            identity = event.get("thread_id")
            if isinstance(identity, str) and identity:
                identities.add(identity)
        completed |= event.get("type") == "turn.completed"
    if len(identities) != 1 or not completed:
        raise ValueError("Codex native thread/resume facts were absent; no fixture was written")
    return identities.pop()


def _environment(home: Path) -> dict[str, str]:
    """Use only the copied authentication record and a disposable session directory."""
    path = os.environ.get("PATH")
    if not path:
        raise ValueError("PATH was unavailable for the isolated Codex capture")
    return {"PATH": path, "HOME": str(home), "CODEX_HOME": str(home / "codex-home"),
            "TERM": "dumb", "NO_COLOR": "1"}


def _seed_command(binary: Path, model: str) -> list[str]:
    return [str(binary), "exec", "--model", model, "--sandbox", "read-only", "--json",
            "--color", "never", "--skip-git-repo-check", "--ignore-user-config",
            "--ignore-rules", "Reply with the exact text GENT_RESUME_SEED_OK and nothing else. Do not use tools."]


def _resume_command(binary: Path, model: str, identity: str) -> list[str]:
    return [str(binary), "exec", "resume", identity, "--model", model, "--json",
            "--skip-git-repo-check", "--ignore-user-config", "--ignore-rules",
            "Reply with the exact text GENT_RESUME_OK and nothing else. Do not use tools."]


def capture_codex_resume(binary: Path, model: str, limit: int, timeout: int) -> None:
    """Prove native same-thread resume using a disposable CODEX_HOME.

    Provider streams and their identity are held only in bounded process memory. The copied
    credential, session files, raw streams, and native identity disappear with the temp directory.
    """
    source_auth = Path.home() / ".codex" / "auth.json"
    if not source_auth.is_file():
        raise ValueError("Codex authentication record was unavailable; no fixture was written")
    with tempfile.TemporaryDirectory(prefix="gent-codex-resume-") as root:
        workspace = Path(root)
        codex_home = workspace / "codex-home"
        codex_home.mkdir(mode=0o700)
        copied_auth = codex_home / "auth.json"
        shutil.copyfile(source_auth, copied_auth)
        copied_auth.chmod(0o600)
        environment = _environment(workspace)
        seed = capture(_seed_command(binary, model), limit, timeout,
                       environment=environment, cwd=str(workspace))
        try:
            identity = _thread_identity(seed)
            if SEED_MARKER not in seed:
                raise ValueError("Codex seed marker was absent; no fixture was written")
            resumed = capture(_resume_command(binary, model, identity), limit, timeout,
                              environment=environment, cwd=str(workspace))
            try:
                resumed_identity = _thread_identity(resumed)
                if RESUME_MARKER not in resumed or resumed_identity != identity:
                    raise ValueError("Codex same-thread resume fact was absent; no fixture was written")
            finally:
                del resumed
        finally:
            del seed
