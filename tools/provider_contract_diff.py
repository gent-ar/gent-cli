from __future__ import annotations

import json
from pathlib import Path


COMPARED = ("cli.json", "protocol.json", "commands.json", "models.json", "frames.json", "launch-probe.json")


def leaves(value: object, path: tuple[str, ...] = ()) -> dict[tuple[str, ...], object]:
    if isinstance(value, dict):
        if not value:
            return {path: {}}
        result: dict[tuple[str, ...], object] = {}
        for key, child in value.items():
            result.update(leaves(child, path + (str(key),)))
        return result
    if isinstance(value, list) and all(isinstance(item, str) for item in value):
        if not value:
            return {path: []}
        return {path + (item,): True for item in value}
    return {path: value}


def load(directory: Path, name: str) -> dict[tuple[str, ...], object]:
    path = directory / name
    if not path.is_file():
        return {}
    return {key: value for key, value in leaves(json.loads(path.read_text(encoding="utf-8"))).items() if key}


def method_of(path: tuple[str, ...], old: dict, new: dict) -> tuple[str, str]:
    method = path[1]
    direction = new.get(("methods", method, "direction")) or old.get(("methods", method, "direction")) or ""
    return method, str(direction)


def classify(name: str, path: tuple[str, ...], change: str, old: dict, new: dict) -> tuple[str, str] | None:
    if name == "cli.json" and len(path) >= 3 and path[1] == "options":
        command = path[0] or "<root>"
        if path[3:] == ("arg",):
            kind = {"added": "flag-added", "removed": "flag-removed", "changed": "flag-shape-changed"}[change]
            return kind, f"{command} {path[2]}"
        return "flag-shape-changed", f"{command} {path[2]} {'/'.join(path[3:])} {change}"
    if name == "cli.json" and len(path) >= 3 and path[1] == "subcommands":
        return f"subcommand-{change}", f"{path[0] or '<root>'} {path[2]}"
    if name == "cli.json":
        return f"cli-{change}", "/".join(path)
    if name == "protocol.json" and path[:1] == ("methods",) and len(path) >= 2:
        method, direction = method_of(path, old, new)
        if len(path) == 3 and path[2] == "direction" and change != "changed":
            if change == "removed":
                return "method-removed", f"{direction} {method}"
            return {"serverRequest": "server-request-new", "serverNotification": "notification-new"}.get(direction, "request-new"), method
        if len(path) >= 4:
            return f"field-{change}", f"{direction} {method} {path[2]} {path[3]}"
        return f"method-{change}", f"{direction} {method} {'/'.join(path[2:])}"
    if name == "protocol.json":
        return f"protocol-{change}", "/".join(path)
    if name == "commands.json" and path[1:] == ("argumentHint",) and change != "changed":
        return ("command-builtin-new" if change == "added" else "command-builtin-removed"), path[0]
    if name == "commands.json":
        return "command-builtin-changed", f"{path[0]} {'/'.join(path[1:])} {change}"
    if name == "frames.json" and path[:1] == ("subtypes",) and len(path) == 2:
        return ("frame-subtype-new" if change == "added" else "frame-subtype-removed"), path[1]
    if name == "models.json":
        return "models-shape-changed", f"{'/'.join(path)} {change}"
    if name == "launch-probe.json":
        if path[-1:] == ("stderr",):
            return ("stderr-warning", "/".join(path[:-1])) if new.get(path) else None
        return "launch-probe-changed", f"{'/'.join(path)} {change}"
    return None


def anchor(name: str, path: tuple[str, ...]) -> tuple[str, ...] | None:
    if name == "cli.json" and len(path) >= 3 and path[1] == "options":
        return path[:3] + ("arg",)
    if name == "protocol.json" and len(path) >= 2 and path[0] == "methods":
        return ("methods", path[1], "direction")
    if name == "commands.json" and path:
        return (path[0], "argumentHint")
    return None


def diff_snapshots(old_directory: Path, new_directory: Path) -> list[str]:
    findings = []
    for name in COMPARED:
        old, new = load(old_directory, name), load(new_directory, name)
        for path in sorted(set(old) | set(new)):
            if path in old and path in new:
                if old[path] == new[path]:
                    continue
                change = "changed"
            else:
                change = "added" if path in new else "removed"
            key = anchor(name, path)
            if key is not None and key != path and (key in old) != (key in new):
                continue
            finding = classify(name, path, change, old, new)
            if finding is not None:
                findings.append(f"{finding[0]} {finding[1]}")
    return sorted(dict.fromkeys(findings))
