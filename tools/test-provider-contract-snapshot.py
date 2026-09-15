#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
from pathlib import Path

from provider_contract_diff import diff_snapshots
from provider_contract_help import parse_help
from provider_contract_schema import claurst_protocol, codex_protocol
from provider_pins import PINS, load_pins


TOOL = Path(__file__).resolve().parent / "provider-contract-snapshot.py"
COMMANDER_HELP = """Usage: claude [options] [command] [prompt]

Arguments:
  prompt                                Your prompt

Options:
  --effort <level>                      Effort level
  --input-format <format>               Input format (only works with --print):
                                        "text" (default), or "stream-json"
                                        (choices: "text", "stream-json")
  --mcp-config <configs...>             Load MCP servers
  -r, --resume [value]                  Resume a conversation
  --safe-mode                           Start with all customizations
  -h, --help                            Display help for command

Commands:
  agents [options]                      Manage background agents
  attach <id>                           Open a background session in this
                                        terminal
  plugin|plugins                        Manage plugins
"""
CLAP_HELP = """Codex CLI

Usage: codex [OPTIONS] [PROMPT]
       codex [OPTIONS] <COMMAND> [ARGS]

Commands:
  exec              Run Codex non-interactively [aliases: e]
  fork              Fork a previous interactive session (picker by default; use --last to fork the
                    most recent)

Options:
  -c, --config <key=value>
          Override a configuration value.
          --not-an-option described inside the help text

      --listen <URL>
          Transport endpoint [possible values: stdio, ws]

  -h, --help
          Print help
"""


def test_commander_help_records_flag_shapes_choices_and_aliases() -> None:
    parsed = parse_help(COMMANDER_HELP)
    assert parsed["usage"] == "claude [options] [command] [prompt]"
    assert parsed["options"]["--effort"] == {"arg": "required", "short": None}
    assert parsed["options"]["--input-format"] == {"arg": "required", "short": None, "choices": ["stream-json", "text"]}
    assert parsed["options"]["--mcp-config"]["arg"] == "required-variadic"
    assert parsed["options"]["--resume"] == {"arg": "optional", "short": "-r"}
    assert parsed["options"]["--safe-mode"]["arg"] == "none"
    assert parsed["subcommands"] == {"agents": [], "attach": [], "plugin": ["plugins"]}


def test_clap_help_ignores_description_lines() -> None:
    parsed = parse_help(CLAP_HELP)
    assert sorted(parsed["options"]) == ["--config", "--help", "--listen"], parsed["options"]
    assert parsed["options"]["--config"] == {"arg": "required", "short": "-c"}
    assert parsed["options"]["--listen"]["choices"] == ["stdio", "ws"]
    assert parsed["subcommands"] == {"exec": ["e"], "fork": []}


def codex_document() -> dict[str, object]:
    method = lambda name, params: {"properties": {"method": {"enum": [name]}, "params": {"$ref": f"#/definitions/v2/{params}"}}}
    return {"definitions": {
        "ClientRequest": {"oneOf": [method("turn/start", "TurnStartParams")]},
        "ClientNotification": {"oneOf": [{"properties": {"method": {"enum": ["initialized"]}}}]},
        "ServerNotification": {"oneOf": [method("turn/completed", "TurnCompletedNotification")]},
        "ServerRequest": {"oneOf": [method("item/tool/call", "ToolCallParams")]},
        "v2": {
            "TurnStartParams": {"type": "object", "properties": {"threadId": {"type": "string"}, "input": {"type": "array", "items": {"$ref": "#/definitions/v2/Node"}}}},
            "TurnStartResponse": {"type": "object", "properties": {"turn": {"anyOf": [{"$ref": "#/definitions/v2/Turn"}, {"type": "null"}]}}},
            "Turn": {"type": "object", "properties": {"status": {"enum": ["completed", "failed"]}}},
            "Node": {"type": "object", "properties": {"child": {"$ref": "#/definitions/v2/Node"}}},
            "TurnCompletedNotification": {"type": "object", "properties": {"threadId": {"type": "string"}}},
            "ToolCallParams": {"type": "object", "properties": {"tool": {"type": "string"}}},
        },
    }}


def test_codex_schema_normalizes_methods_fields_and_results() -> None:
    methods = codex_protocol(codex_document())["methods"]
    assert {name: entry["direction"] for name, entry in methods.items()} == {
        "initialized": "clientNotification", "item/tool/call": "serverRequest",
        "turn/completed": "serverNotification", "turn/start": "clientRequest",
    }
    start = methods["turn/start"]
    assert start["params"]["/threadId"] == "string"
    assert start["params"]["/input/[]"] == "object"
    assert start["params"]["/input/[]/child"] == "ref:Node"
    assert "/input/[]/child/child" not in start["params"]
    assert start["result"]["/turn"] == "null|object"
    assert start["result"]["/turn/status"] == "enum:completed|failed"
    assert methods["item/tool/call"]["result"] == {}
    assert "result" not in methods["turn/completed"]


def test_claurst_source_scan_records_served_and_emitted_surface() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        acp = root / "checkout/src-rust/crates/acp/src"
        schema = root / "schema/src/v1"
        acp.mkdir(parents=True)
        schema.mkdir(parents=True)
        (acp / "server.rs").write_text('match method {\n    "initialize" => {\n ok\n }\n    "session/load" => {\n Err(acp::Error::method_not_found())\n }\n}\nasync fn handle_notification() {\n    "session/cancel" => {}\n}\n.load_session(false).prompt_capabilities(acp::PromptCapabilities::new().image(true))\n')
        (acp / "prompt.rs").write_text("acp::SessionUpdate::AgentMessageChunk(chunk)\nacp::SessionUpdate::ToolCall(call)\n")
        (schema / "client.rs").write_text('pub enum SessionUpdate {\n    AgentMessageChunk(ContentChunk),\n    AvailableCommandsUpdate(AvailableCommandsUpdate),\n}\nconst A_METHOD_NAME: &str = "session/request_permission";\n')
        (schema / "agent.rs").write_text('const B_METHOD_NAME: &str = "session/set_mode";\n')
        protocol = claurst_protocol(root / "checkout", root / "schema")
    assert protocol["servedRequests"] == ["initialize"]
    assert protocol["rejectedRequests"] == ["session/load"]
    assert protocol["servedNotifications"] == ["session/cancel"]
    assert protocol["emittedSessionUpdates"] == ["agent_message_chunk", "tool_call"]
    assert protocol["schemaSessionUpdates"] == ["agent_message_chunk", "available_commands_update"]
    assert protocol["initialize"] == {"loadSession": False, "promptCapabilities": ["image"]}


def write(directory: Path, files: dict[str, object]) -> Path:
    directory.mkdir(parents=True)
    for name, value in files.items():
        (directory / name).write_text(json.dumps(value), encoding="utf-8")
    return directory


def snapshot(**changes: object) -> dict[str, object]:
    files = {
        "cli.json": {"<root>": {"usage": "claude", "options": {"--effort": {"arg": "required", "short": None}, "--permission-mode": {"arg": "required", "short": None, "choices": ["manual", "plan"]}, "--resume": {"arg": "optional", "short": "-r"}}, "subcommands": {"auth": []}}},
        "protocol.json": {"methods": {"turn/start": {"direction": "clientRequest", "params": {"/threadId": "string", "/input": "array"}, "result": {}}, "turn/failed": {"direction": "serverNotification", "params": {"/turn": "object"}}}},
        "commands.json": {"compact": {"argumentHint": "<instructions>", "aliases": []}},
        "frames.json": {"subtypes": ["can_use_tool", "init"]},
        "launch-probe.json": {"chat": {"handshake": "success", "stderr": ""}},
    }
    files.update(changes)
    return files


def test_diff_classifies_every_contract_change() -> None:
    new = snapshot(
        **{
            "cli.json": {"<root>": {"usage": "claude", "options": {"--effort": {"arg": "optional", "short": None}, "--permission-mode": {"arg": "required", "short": None, "choices": ["ask", "plan"]}}, "subcommands": {"auth": [], "login": []}}},
            "protocol.json": {"methods": {"turn/start": {"direction": "clientRequest", "params": {"/threadId": "integer"}, "result": {}}, "thread/goal/updated": {"direction": "serverNotification", "params": {"/goal": "object"}}, "item/tool/ask": {"direction": "serverRequest", "params": {}}}},
            "commands.json": {"compact": {"argumentHint": "<instructions>", "aliases": []}, "rewind": {"argumentHint": "", "aliases": ["undo"]}},
            "frames.json": {"subtypes": ["init", "turn_preempted"]},
            "launch-probe.json": {"chat": {"handshake": "success", "stderr": "Warning: Unknown --permission-mode value 'manual'"}},
        }
    )
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        findings = diff_snapshots(write(root / "old", snapshot()), write(root / "new", new))
    expected = [
        "command-builtin-new rewind",
        "field-changed clientRequest turn/start params /threadId",
        "field-removed clientRequest turn/start params /input",
        "flag-removed <root> --resume",
        "flag-shape-changed <root> --effort",
        "flag-shape-changed <root> --permission-mode choices/ask added",
        "flag-shape-changed <root> --permission-mode choices/manual removed",
        "frame-subtype-new turn_preempted",
        "frame-subtype-removed can_use_tool",
        "method-removed serverNotification turn/failed",
        "notification-new thread/goal/updated",
        "server-request-new item/tool/ask",
        "stderr-warning chat",
        "subcommand-added <root> login",
    ]
    assert findings == expected, findings


def test_claude_frame_subtypes_are_read_from_bundled_literals() -> None:
    spec = importlib.util.spec_from_file_location("snapshot_tool", TOOL)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    bundle = b'x={type:"system",subtype:"peer_message_hold"};if(r.subtype==="can_use_tool")y=c({subtype:k("turn_preempted")});z={subtype:"X"}'
    assert sorted(module.CLAUDE_SUBTYPE_LITERAL.findall(bundle)) == [b"can_use_tool", b"peer_message_hold", b"turn_preempted"]


def test_diff_cli_exit_codes() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        old = write(root / "old", snapshot())
        same = write(root / "same", snapshot())
        changed = write(root / "changed", snapshot(**{"commands.json": {}}))
        unchanged = subprocess.run([sys.executable, TOOL, "--diff", old, same], capture_output=True, text=True)
        drifted = subprocess.run([sys.executable, TOOL, "--diff", old, changed], capture_output=True, text=True)
        refused = subprocess.run([sys.executable, TOOL, "codex"], capture_output=True, text=True)
    assert unchanged.returncode == 0 and unchanged.stdout.strip() == "no contract changes", unchanged
    assert drifted.returncode == 1 and drifted.stdout.strip() == "command-builtin-removed compact", drifted
    assert refused.returncode != 0 and "--executable" in refused.stderr, refused


def test_repository_snapshots_cover_the_pinned_versions() -> None:
    pins = load_pins(PINS)
    for provider, required in {"claude": ("cli.json", "protocol.json", "commands.json", "models.json", "frames.json", "launch-probe.json"), "codex": ("cli.json", "protocol.json", "launch-probe.json"), "claurst": ("protocol.json",)}.items():
        directory = PINS.parent / provider / pins["providers"][provider]["version"]
        for name in ("source.json", *required):
            assert (directory / name).is_file(), directory / name
        probes = json.loads((directory / "launch-probe.json").read_text()) if provider != "claurst" else {}
        for name, probe in probes.items():
            assert probe["handshake"] == "success" and probe["stderr"] == "", (provider, name, probe)


if __name__ == "__main__":
    for name, test in sorted(globals().items()):
        if name.startswith("test_"):
            test()
    print("provider contract snapshot checks passed")
