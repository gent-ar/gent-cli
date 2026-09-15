#!/usr/bin/env python3
import json
import os
import select
import signal
import sys
import time
import uuid

ROOT = os.path.dirname(os.path.abspath(__file__))
SESSIONS = os.path.join(ROOT, "sessions")
ARGV = sys.argv[1:]
PERSISTENT = "--no-session-persistence" not in ARGV
REPLAY = "--replay-user-messages" in ARGV
PENDING = []
BACKGROUND_SUBAGENT = os.path.join(ROOT, "background-subagent.jsonl")
BUFFER = bytearray()


def emit(frame):
    sys.stdout.write(json.dumps(frame) + "\n")
    sys.stdout.flush()


def read_frame(timeout):
    while b"\n" not in BUFFER:
        ready, _, _ = select.select([0], [], [], timeout)
        if not ready:
            return None
        chunk = os.read(0, 65536)
        if not chunk:
            sys.exit(0)
        BUFFER.extend(chunk)
    index = BUFFER.index(b"\n")
    line = bytes(BUFFER[:index])
    del BUFFER[: index + 1]
    return json.loads(line)


def transcript(session_id):
    return os.path.join(SESSIONS, session_id + ".jsonl")


def history(session_id):
    if not PERSISTENT:
        return []
    with open(transcript(session_id)) as handle:
        return [json.loads(line) for line in handle]


def remember(session_id, text):
    if PERSISTENT:
        with open(transcript(session_id), "a") as handle:
            handle.write(json.dumps(text) + "\n")


def user_text(frame):
    blocks = frame["message"]["content"]
    return "".join(block.get("text", "") for block in blocks if block.get("type") == "text")


def accept(session_id, frame):
    if REPLAY:
        replay = {"type": "user", "message": frame["message"], "session_id": session_id}
        replay.update({"isReplay": True, "uuid": frame.get("uuid") or str(uuid.uuid4())})
        emit(replay)
    remember(session_id, user_text(frame))


def chunks(session_id, text):
    if "LONG" in text:
        return ["lighthouse "] * 400
    codes = sorted({word for prior in history(session_id) for word in prior.split() if word.startswith("CODE-")})
    return ["recall: " + (",".join(codes) or "nothing")]


def result(session_id, failed):
    emit({
        "type": "result",
        "subtype": "error_during_execution" if failed else "success",
        "is_error": failed,
        "session_id": session_id,
        "usage": {"input_tokens": 10, "cache_creation_input_tokens": 8680, "cache_read_input_tokens": 13607, "output_tokens": 113},
    })


def request_permission(session_id):
    PENDING.append("permission-" + str(uuid.uuid4()))
    request = {"subtype": "can_use_tool", "tool_name": "Bash", "tool_use_id": "toolu-" + PENDING[0], "input": {}}
    emit({"type": "control_request", "request_id": PENDING[0], "request": request, "session_id": session_id})


def propose_plan(session_id):
    plan = "1. Create README.md\n2. Add a usage section"
    tool = {"type": "tool_use", "id": "toolu-plan", "name": "ExitPlanMode", "input": {"plan": plan}}
    emit({"type": "assistant", "message": {"content": [tool]}, "session_id": session_id})
    PENDING.append("plan-" + str(uuid.uuid4()))
    request = {"subtype": "can_use_tool", "tool_name": "ExitPlanMode", "tool_use_id": "toolu-plan", "input": {"plan": plan}}
    emit({"type": "control_request", "request_id": PENDING[0], "request": request, "session_id": session_id})


def background_subagent(session_id):
    with open(BACKGROUND_SUBAGENT) as handle:
        for line in handle:
            frame = json.loads(line.replace("session-bg", session_id))
            emit(frame)
            if frame["type"] == "result":
                time.sleep(0.5)


def delta(session_id, text):
    event = {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}
    emit({"type": "stream_event", "event": event, "session_id": session_id})


def slow_steps(session_id):
    for step in range(4):
        delta(session_id, "step %d " % step)
        deadline = time.time() + 0.4
        while time.time() < deadline:
            frame = read_frame(max(0.0, deadline - time.time()))
            if frame is not None and frame.get("type") == "user":
                accept(session_id, frame)


def turn(session_id, text):
    if "SUBAGENT" in text:
        return background_subagent(session_id)
    announced = str(uuid.uuid4()) if "ROTATE" in text else session_id
    emit({"type": "system", "subtype": "init", "session_id": announced})
    emit({"type": "stream_event", "event": {"type": "message_start", "message": {}}, "session_id": session_id})
    if "PERMISSION" in text:
        return request_permission(session_id)
    if "PLAN" in text and "plan" in ARGV:
        return propose_plan(session_id)
    if "HOLD" in text:
        delta(session_id, "holding ")
        while True:
            time.sleep(1)
    if "SLOW" in text:
        slow_steps(session_id)
    if "RACE" in text:
        delta(session_id, "racing ")
    late = read_frame(None) if "RACE" in text else None
    reply = chunks(session_id, text)
    for chunk in reply:
        delta(session_id, chunk)
        time.sleep(0.02)
    content = [{"type": "text", "text": "".join(reply)}]
    emit({"type": "assistant", "message": {"content": content}, "session_id": session_id})
    result(session_id, False)
    if late is not None:
        accept(session_id, late)
        turn(session_id, user_text(late))


def main():
    os.makedirs(SESSIONS, exist_ok=True)
    with open(os.path.join(ROOT, "launches.jsonl"), "a") as handle:
        handle.write(json.dumps(ARGV) + "\n")
    resumed = ARGV[ARGV.index("--resume") + 1] if "--resume" in ARGV else None
    chosen = ARGV[ARGV.index("--session-id") + 1] if "--session-id" in ARGV else None
    if resumed is not None and os.path.exists(os.path.join(ROOT, "resume-api-error")):
        read_frame(None)
        sys.stderr.write("API Error: 401 authentication_error\n")
        emit({"type": "result", "subtype": "error_during_execution", "duration_ms": 640, "duration_api_ms": 612, "is_error": True, "num_turns": 0, "session_id": resumed, "errors": ["API Error: 401"]})
        sys.exit(1)
    if resumed is not None and not os.path.exists(transcript(resumed)):
        read_frame(None)
        sys.stderr.write("No conversation found with session ID: " + resumed + "\n")
        emit({"type": "result", "subtype": "error_during_execution", "duration_ms": 0, "duration_api_ms": 0, "is_error": True, "num_turns": 0, "session_id": resumed, "errors": ["No conversation found with session ID: " + resumed]})
        sys.exit(1)
    if chosen is not None and os.path.exists(transcript(chosen)):
        sys.stderr.write("Error: Session ID " + chosen + " is already in use.\n")
        sys.exit(1)
    session_id = resumed or chosen or str(uuid.uuid4())
    if resumed is None and PERSISTENT:
        open(transcript(session_id), "a").close()

    def interrupted(*_):
        for request_id in PENDING:
            emit({"type": "control_cancel_request", "request_id": request_id})
        result(session_id, True)
        sys.exit(0)

    signal.signal(signal.SIGINT, interrupted)
    while True:
        frame = read_frame(None)
        if frame.get("type") == "user" and PENDING:
            accept(session_id, frame)
        elif frame.get("type") == "user":
            accept(session_id, frame)
            turn(session_id, user_text(frame))
        elif frame.get("type") == "control_response" and PENDING:
            PENDING.clear()
            result(session_id, False)


main()
