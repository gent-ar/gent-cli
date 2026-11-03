"""Producer policy; the host floor is independent of target/GUI version."""
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent

def compatibility(root=ROOT):
    policy = json.loads((root / "runtime-release-policy.json").read_text(encoding="utf-8"))
    expected = {"schemaVersion", "minimumCompatibleHostVersion", "protocolMin", "protocolMax", "schemaMin"}
    if not isinstance(policy, dict) or set(policy) != expected or type(policy["schemaVersion"]) is not int or policy["schemaVersion"] != 1:
        raise ValueError("runtime release compatibility policy is invalid")
    floor = policy["minimumCompatibleHostVersion"]
    if not isinstance(floor, str) or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", floor) is None:
        raise ValueError("minimum compatible native host version is invalid")
    source = (root / "crates/gent-store/src/lib.rs").read_text(encoding="utf-8")
    versions = re.findall(r"(?m)^pub const CURRENT_LEDGER_SCHEMA_VERSION:\s*u32\s*=\s*([0-9]+)\s*;", source)
    if len(versions) != 1:
        raise ValueError("the authoritative current ledger schema version is unavailable")
    maximum = int(versions[0])
    for field in ("protocolMin", "protocolMax", "schemaMin"):
        if type(policy[field]) is not int or policy[field] < 1:
            raise ValueError("runtime release compatibility bounds are invalid")
    if policy["protocolMin"] > policy["protocolMax"] or policy["schemaMin"] > maximum:
        raise ValueError("runtime release compatibility ranges are invalid")
    return {**policy, "schemaMax": maximum}

if __name__ == "__main__":
    policy = compatibility()
    print(policy["protocolMin"], policy["protocolMax"], policy["schemaMin"],
          policy["schemaMax"], policy["minimumCompatibleHostVersion"])
