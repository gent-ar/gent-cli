from __future__ import annotations

import hashlib
import hmac


NESTED_KEY_SALT = b"gent-ordinary-authority-nested-signing-keys-v1"
NESTED_KEY_PURPOSES = ("compatibility", "package-policy", "evidence")
SEED_BYTES = 32


def hkdf_sha256(key_material: bytes, info: bytes, length: int, salt: bytes = b"") -> bytes:
    if not 0 < length <= 255 * hashlib.sha256().digest_size:
        raise ValueError("HKDF-SHA256 output length is out of range")
    pseudorandom_key = hmac.new(salt or bytes(hashlib.sha256().digest_size), key_material, hashlib.sha256).digest()
    output, block = b"", b""
    for counter in range(1, 256):
        if len(output) >= length:
            break
        block = hmac.new(pseudorandom_key, block + info + bytes([counter]), hashlib.sha256).digest()
        output += block
    return output[:length]


def nested_signing_keys(root_key_id: str, root_seed: bytes) -> dict[str, tuple[str, bytes]]:
    if len(root_seed) != SEED_BYTES:
        raise ValueError("root signing seed must be 32 bytes")
    return {
        purpose: (f"{root_key_id}.{purpose}", hkdf_sha256(root_seed, purpose.encode("ascii"), SEED_BYTES, NESTED_KEY_SALT))
        for purpose in NESTED_KEY_PURPOSES
    }
