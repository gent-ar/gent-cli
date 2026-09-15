from __future__ import annotations

import re
from pathlib import Path


MAX_DEPTH = 6
DIRECTIONS = {
    "ClientRequest": "clientRequest",
    "ClientNotification": "clientNotification",
    "ServerNotification": "serverNotification",
    "ServerRequest": "serverRequest",
}


def pointer(document: dict[str, object], reference: str) -> tuple[dict[str, object], str]:
    node: object = document
    parts = reference.removeprefix("#/").split("/")
    for part in parts:
        node = node[part]
    return node, "/".join(parts[:-1])


def merge(fields: dict[str, set[str]], path: str, kind: str) -> None:
    fields.setdefault(path, set()).add(kind)


def walk(document, schema, path, fields, depth, stack) -> None:
    if not isinstance(schema, dict) or depth > MAX_DEPTH:
        return
    reference = schema.get("$ref")
    if isinstance(reference, str):
        name = reference.rsplit("/", 1)[-1]
        if name in stack:
            merge(fields, path, f"ref:{name}")
            return
        target, _ = pointer(document, reference)
        walk(document, target, path, fields, depth, stack | {name})
        return
    for key in ("allOf", "anyOf", "oneOf"):
        for variant in schema.get(key, []):
            walk(document, variant, path, fields, depth, stack)
    if "enum" in schema:
        merge(fields, path, "enum:" + "|".join(sorted(map(str, schema["enum"]))))
    if "const" in schema:
        merge(fields, path, f"enum:{schema['const']}")
    kinds = schema.get("type")
    for kind in kinds if isinstance(kinds, list) else [kinds] if kinds else []:
        merge(fields, path, kind)
    for name, child in schema.get("properties", {}).items():
        walk(document, child, f"{path}/{name}", fields, depth + 1, stack)
    if isinstance(schema.get("items"), dict):
        walk(document, schema["items"], f"{path}/[]", fields, depth + 1, stack)
    if isinstance(schema.get("additionalProperties"), dict):
        walk(document, schema["additionalProperties"], f"{path}/{{}}", fields, depth + 1, stack)


def field_tree(document: dict[str, object], reference: str | None) -> dict[str, str]:
    if reference is None:
        return {}
    fields: dict[str, set[str]] = {}
    walk(document, {"$ref": reference}, "", fields, 0, frozenset())
    return {path or "/": "|".join(sorted(kinds)) for path, kinds in sorted(fields.items())}


def response_reference(document: dict[str, object], params_reference: str | None) -> str | None:
    if params_reference is None or not params_reference.endswith("Params"):
        return None
    candidate = params_reference.removesuffix("Params") + "Response"
    try:
        pointer(document, candidate)
    except KeyError:
        return None
    return candidate


def codex_protocol(document: dict[str, object]) -> dict[str, object]:
    methods: dict[str, dict[str, object]] = {}
    definitions = document["definitions"]
    for union, direction in DIRECTIONS.items():
        for variant in definitions[union]["oneOf"]:
            properties = variant["properties"]
            params = properties.get("params", {}).get("$ref")
            for method in properties["method"]["enum"]:
                entry: dict[str, object] = {"direction": direction, "params": field_tree(document, params)}
                if direction.endswith("Request"):
                    entry["result"] = field_tree(document, response_reference(document, params))
                methods[method] = entry
    return {"methods": dict(sorted(methods.items()))}


def snake(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def rust_enum_variants(source: str, enum: str) -> list[str]:
    match = re.search(r"pub enum " + enum + r"\s*\{(.*?)\n\}", source, re.S)
    if match is None:
        raise ValueError(f"enum {enum} not found")
    return sorted(snake(name) for name in re.findall(r"^\s{4}([A-Z][A-Za-z]+)[\s(,{]", match.group(1), re.M))


def claurst_protocol(checkout: Path, acp_schema: Path) -> dict[str, object]:
    server = (checkout / "src-rust/crates/acp/src/server.rs").read_text(encoding="utf-8")
    prompt = (checkout / "src-rust/crates/acp/src/prompt.rs").read_text(encoding="utf-8")
    request_block = server.split("async fn handle_notification", 1)
    requests = re.findall(r"^\s+\"([a-z/_$]+)\" =>", request_block[0], re.M)
    notifications = re.findall(r"^\s+\"([a-z/_$]+)\" =>", request_block[1], re.M) if len(request_block) > 1 else []
    rejected = re.findall(r"\"([a-z/_]+)\" =>\s*\{[^}]*?method_not_found", request_block[0], re.S)
    client = (acp_schema / "src/v1/client.rs").read_text(encoding="utf-8")
    agent = (acp_schema / "src/v1/agent.rs").read_text(encoding="utf-8")
    return {
        "servedRequests": sorted(set(requests) - set(rejected)),
        "rejectedRequests": sorted(rejected),
        "servedNotifications": sorted(notifications),
        "emittedSessionUpdates": sorted({snake(name) for name in re.findall(r"SessionUpdate::([A-Z][A-Za-z]+)", prompt)}),
        "schemaSessionUpdates": rust_enum_variants(client, "SessionUpdate"),
        "schemaAgentMethods": sorted(set(re.findall(r"_(?:METHOD_NAME|NOTIFICATION): &str = \"([^\"]+)\"", agent))),
        "schemaClientMethods": sorted(set(re.findall(r"_(?:METHOD_NAME|NOTIFICATION): &str = \"([^\"]+)\"", client))),
        "initialize": {
            "loadSession": "load_session(true)" in server,
            "promptCapabilities": sorted(set(re.findall(r"\.(\w+)\(", "".join(re.findall(r"PromptCapabilities::new\(\)((?:\s*\.\w+\([^)]*\))*)", server))))),
        },
    }
