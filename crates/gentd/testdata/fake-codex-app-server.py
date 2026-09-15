#!/usr/bin/env python3
import json
import os
import re
import sys
import uuid

ROOT = os.path.dirname(os.path.abspath(__file__))
THREADS = os.path.join(ROOT, "threads")
LAUNCH = str(uuid.uuid4())
LOADED = set()
PENDING = {}


def emit(frame):
    frame["jsonrpc"] = "2.0"
    sys.stdout.write(json.dumps(frame) + "\n")
    sys.stdout.flush()


def log(method, params):
    with open(os.path.join(ROOT, "requests.jsonl"), "a") as handle:
        handle.write(json.dumps({"launch": LAUNCH, "method": method, "params": params}) + "\n")


def rollout(thread_id):
    return os.path.join(THREADS, thread_id + ".jsonl")


def turns(thread_id):
    with open(rollout(thread_id)) as handle:
        return [json.loads(line) for line in handle]


def persist(thread_id, turn):
    with open(rollout(thread_id), "a") as handle:
        handle.write(json.dumps(turn) + "\n")


def reply(thread_id, text):
    if "LONG" in text:
        return ["lighthouse " * 180] * 48
    history = " ".join(turn["user"] + " " + turn["agent"] for turn in turns(thread_id))
    codes = sorted(set(re.findall(r"CODE-[A-Z0-9]+", history)))
    return ["recall: " + (",".join(codes) or "nothing")]


def complete(thread_id, turn_id, status):
    emit({"method": "turn/completed", "params": {"threadId": thread_id, "turn": {"id": turn_id, "status": status}}})


def thread_start(request_id, params):
    thread_id = str(uuid.uuid4())
    open(rollout(thread_id), "a").close()
    LOADED.add(thread_id)
    emit({"id": request_id, "result": {"thread": {"id": thread_id, "turns": []}}})
    emit({"method": "thread/started", "params": {"thread": {"id": thread_id}}})


def thread_turns_list(request_id, params):
    thread_id = params["threadId"]
    if not os.path.exists(rollout(thread_id)):
        message = "invalid paginated history lineage for " + thread_id + ": missing source rollout"
        return emit({"id": request_id, "error": {"code": -32600, "message": message}})
    emit({"id": request_id, "result": {"data": turns(thread_id)[-1:], "nextCursor": None}})


def thread_resume(request_id, params):
    thread_id = params["threadId"]
    if os.path.exists(os.path.join(ROOT, "resume-rejected")):
        return emit({"id": request_id, "error": {"code": -32600, "message": "resume rejected"}})
    if not os.path.exists(rollout(thread_id)):
        message = "no rollout found for thread id " + thread_id
        return emit({"id": request_id, "error": {"code": -32600, "message": message}})
    LOADED.add(thread_id)
    history = [] if params.get("excludeTurns") else turns(thread_id)
    emit({"id": request_id, "result": {"thread": {"id": thread_id, "turns": history}}})


def turn_start(request_id, params):
    thread_id = params["threadId"]
    if thread_id not in LOADED:
        return emit({"id": request_id, "error": {"code": -32600, "message": "thread not found: " + thread_id}})
    text = "".join(block.get("text", "") for block in params["input"] if block.get("type") == "text")
    turn_id = str(uuid.uuid4())
    emit({"id": request_id, "result": {"turn": {"id": turn_id, "status": "inProgress"}}})
    emit({"method": "turn/started", "params": {"threadId": thread_id, "turn": {"id": turn_id, "status": "inProgress"}}})
    chunks = (["working on ", "CODE-UNSAID"] if "SECRET" in text else ["working"]) if "WAIT" in text else reply(thread_id, text)
    for chunk in chunks:
        delta = {"threadId": thread_id, "turnId": turn_id, "itemId": "message-" + turn_id, "delta": chunk}
        emit({"method": "item/agentMessage/delta", "params": delta})
    if "WAIT" in text or "HOLD" in text or "LATE" in text:
        persist(thread_id, {"id": turn_id, "user": text, "agent": ""})
        PENDING[turn_id] = {"thread": thread_id, "text": text, "steers": 0}
    else:
        persist(thread_id, {"id": turn_id, "user": text, "agent": "".join(chunks)})
        complete(thread_id, turn_id, "completed")


def turn_steer(request_id, params):
    turn_id = params["expectedTurnId"]
    active = PENDING.get(turn_id)
    if active is None or active["thread"] != params["threadId"]:
        return emit({"id": request_id, "error": {"code": -32600, "message": "no active turn to steer"}})
    if "LATE" in active["text"]:
        complete(PENDING.pop(turn_id)["thread"], turn_id, "completed")
        return emit({"id": request_id, "error": {"code": -32600, "message": "no active turn to steer"}})
    emit({"id": request_id, "result": {"turnId": turn_id}})
    if "HOLD" in active["text"]:
        return
    text = "".join(block.get("text", "") for block in params["input"] if block.get("type") == "text")
    item = {"type": "userMessage", "id": str(uuid.uuid4()), "clientId": params.get("clientUserMessageId"), "content": params["input"]}
    emit({"method": "item/started", "params": {"threadId": active["thread"], "turnId": turn_id, "item": item}})
    persist(active["thread"], {"id": turn_id + "-steer", "user": text, "agent": ""})
    active["steers"] += 1
    if active["steers"] >= (2 if "TWICE" in active["text"] else 1):
        delta = {"threadId": active["thread"], "turnId": turn_id, "itemId": "message-" + turn_id, "delta": " steered"}
        emit({"method": "item/agentMessage/delta", "params": delta})
        complete(PENDING.pop(turn_id)["thread"], turn_id, "completed")


def thread_inject_items(request_id, params):
    thread_id = params["threadId"]
    if thread_id not in LOADED:
        return emit({"id": request_id, "error": {"code": -32600, "message": "thread not found: " + thread_id}})
    for item in params["items"]:
        text = "".join(part.get("text", "") for part in item.get("content", []))
        persist(thread_id, {"id": str(uuid.uuid4()), "user": "", "agent": text})
    emit({"id": request_id, "result": {}})


def turn_interrupt(request_id, params):
    emit({"id": request_id, "result": {}})
    active = PENDING.pop(params["turnId"], None)
    if active is not None:
        complete(active["thread"], params["turnId"], "interrupted")


def thread_compact_start(request_id, params):
    thread_id = params["threadId"]
    if thread_id not in LOADED:
        return emit({"id": request_id, "error": {"code": -32600, "message": "thread not found: " + thread_id}})
    turn_id = str(uuid.uuid4())
    item = {"type": "contextCompaction", "id": "compaction-" + turn_id}
    emit({"id": request_id, "result": {}})
    emit({"method": "turn/started", "params": {"threadId": thread_id, "turn": {"id": turn_id, "status": "inProgress"}}})
    emit({"method": "item/started", "params": {"threadId": thread_id, "turnId": turn_id, "item": item}})
    emit({"method": "item/completed", "params": {"threadId": thread_id, "turnId": turn_id, "item": dict(item, status="completed")}})
    emit({"method": "thread/compacted", "params": {"threadId": thread_id, "turnId": turn_id}})
    complete(thread_id, turn_id, "completed")


HANDLERS = {
    "initialize": lambda request_id, _: emit({"id": request_id, "result": {"userAgent": "fake-codex"}}),
    "thread/start": thread_start,
    "thread/resume": thread_resume,
    "thread/turns/list": thread_turns_list,
    "turn/start": turn_start,
    "turn/interrupt": turn_interrupt,
    "turn/steer": turn_steer,
    "thread/inject_items": thread_inject_items,
    "thread/compact/start": thread_compact_start,
}


def main():
    os.makedirs(THREADS, exist_ok=True)
    log("launch", {"argv": sys.argv[1:], "managed": sorted(name for name in os.environ if name.startswith("CODEX_MANAGED"))})
    for line in sys.stdin:
        frame = json.loads(line)
        method = frame.get("method")
        log(method, frame.get("params"))
        if method in HANDLERS and "id" in frame:
            HANDLERS[method](frame["id"], frame.get("params") or {})


main()
