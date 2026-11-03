import hashlib
import json
from pathlib import Path


def verify_windows_build_binding(path, target, files, sandbox):
    from windows_sandbox_package import ordinary, sha, unique_object
    binding = json.loads(ordinary(path).read_text(encoding="utf-8"), object_pairs_hook=unique_object)
    if (not isinstance(binding, dict) or binding.get("schemaVersion") != 1
            or binding.get("phase") != "built" or binding.get("target") != target
            or binding.get("sandboxFiles") != sandbox.expected):
        raise ValueError("Windows sandbox build binding does not match the packaged runtime")
    binaries_expected = binding.get("binaries")
    expected_names = {"gent.exe", "gentd.exe", "gent-launcher.exe"}
    if (not isinstance(binaries_expected, dict) or set(binaries_expected) != expected_names
            or {p.name for p in files} != expected_names
            or any(sha(ordinary(p)) != binaries_expected[p.name] for p in files)):
        raise ValueError("Windows sandbox build binding does not match the packaged binaries")
    pins = binding.get("cargoEnvironment")
    pin_files = {
        "GENT_WINDOWS_SANDBOX_HELPER_SHA256": "runtime/windows/provider-helper.exe",
        "GENT_WINDOWS_PROVIDER_RESOURCES_SHA256": "runtime/windows/provider-resources.json",
        "GENT_WINDOWS_EXTERNAL_PROXY_POLICY_SHA256": "runtime/windows/reviewed-external-proxy-policy.json",
        "GENT_WINDOWS_GIT_BASH_SHA256": "runtime/windows/git/usr/bin/bash.exe",
    }
    model = "runtime/claurst/model-server/model-runtime.json"
    if model in sandbox.expected:
        pin_files["GENT_CONTAINED_MODEL_MANIFEST_SHA256"] = model
    if (not isinstance(pins, dict) or set(pins) != set(pin_files)
            or any(pins[key] != sandbox.expected.get(name) for key, name in pin_files.items())):
        raise ValueError("Windows sandbox build binding has mismatched compile-time pins")
    return binaries_expected


def require_windows_provider_coverage(runtime, expected_digest):
    from windows_sandbox_package import ordinary, reviewed_providers, unique_object
    descriptor = ordinary(Path(runtime) / "reviewed-external-proxy-policy.json")
    data = descriptor.read_bytes()
    if hashlib.sha256(data).hexdigest() != expected_digest:
        raise ValueError("Windows sandbox descriptor changed during provider coverage check")
    policy = json.loads(data.decode("utf-8"), object_pairs_hook=unique_object)
    if set(reviewed_providers(policy)) != {"codex", "claude", "claurst"}:
        raise ValueError("Windows release sandbox must cover Codex, Claude and Claurst")
