#!/usr/bin/env python3
import json
import os
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
LOG = os.path.join(ROOT, "model-list-requests.jsonl")
PAGES = {
    None: {
        "data": [
            {"id": "gpt-6-astra", "model": "gpt-6-astra", "displayName": "GPT-6-Astra", "description": "Frontier", "hidden": False, "isDefault": True, "defaultReasoningEffort": "medium", "supportedReasoningEfforts": [{"reasoningEffort": "low", "description": ""}, {"reasoningEffort": "medium", "description": ""}, {"reasoningEffort": "ultra", "description": ""}, {"reasoningEffort": "hyper", "description": ""}]},
            {"id": "gpt-internal", "model": "gpt-internal", "displayName": "Internal", "description": "", "hidden": True, "isDefault": False, "defaultReasoningEffort": "low", "supportedReasoningEfforts": []},
        ],
        "nextCursor": "page-2",
    },
    "page-2": {
        "data": [
            {"id": "gpt-5.6-luna", "model": "gpt-5.6-luna", "displayName": "GPT-5.6-Luna", "description": "", "hidden": False, "isDefault": False, "defaultReasoningEffort": "hyper", "supportedReasoningEfforts": [{"reasoningEffort": "xhigh", "description": ""}]},
        ],
        "nextCursor": None,
    },
}

for line in sys.stdin:
    request = json.loads(line)
    with open(LOG, "a") as handle:
        handle.write(json.dumps(request) + "\n")
    if request.get("method") == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": {"userAgent": "fake"}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "method": "remoteControl/status/changed", "params": {}}), flush=True)
    elif request.get("method") == "model/list":
        if os.path.exists(os.path.join(ROOT, "model-list-error")):
            print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32000, "message": "not signed in"}}), flush=True)
            continue
        page = PAGES[request["params"].get("cursor")]
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": page}), flush=True)
