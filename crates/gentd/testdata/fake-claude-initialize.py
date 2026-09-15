#!/usr/bin/env python3
import json
import os
import sys

for line in sys.stdin:
    request = json.loads(line)
    if request.get("type") != "control_request" or request["request"].get("subtype") != "initialize":
        continue
    print(json.dumps({"type": "system", "subtype": "hook_started"}), flush=True)
    print(json.dumps({"type": "control_response", "response": {"subtype": "success", "request_id": "other", "response": {}}}), flush=True)
    print(json.dumps({"type": "control_response", "response": {"subtype": "success", "request_id": request["request_id"], "response": {
        "commands": [
            {"name": "context", "description": "Show context usage", "argumentHint": ""},
            {"name": "clear", "description": "Clear conversation", "aliases": ["new", "reset"]},
            {"name": "config", "description": "Open settings", "aliases": ["settings"]},
            {"name": "workspace-skill", "description": "Skill loaded from " + os.getcwd()}
        ],
        "models": [
            {"value": "default", "resolvedModel": "claude-opus-5[1m]", "displayName": "Default (recommended)", "description": "Opus 5 with 1M context", "supportsEffort": True, "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"]},
            {"value": "sonnet", "resolvedModel": "claude-sonnet-5", "displayName": "Sonnet", "description": "Sonnet 5", "supportedEffortLevels": ["low", "medium", "high"]},
            {"value": "haiku", "resolvedModel": "claude-haiku-4-5", "displayName": "Haiku", "description": "Haiku 4.5"},
            {"value": "", "displayName": "Broken"}
        ],
        "account": {"email": "user@example.test"}
    }}}), flush=True)
