#!/usr/bin/env python3
from __future__ import annotations

import hashlib

from authority_keys import NESTED_KEY_PURPOSES, NESTED_KEY_SALT, SEED_BYTES, hkdf_sha256, nested_signing_keys


ROOT_SEED = hashlib.sha256(b"gent-authority-keys-test-root").digest()
OTHER_SEED = hashlib.sha256(b"gent-authority-keys-test-other-root").digest()


def test_hkdf_matches_the_rfc_5869_sha256_vectors() -> None:
    derived = hkdf_sha256(
        bytes.fromhex("0b" * 22),
        bytes.fromhex("f0f1f2f3f4f5f6f7f8f9"),
        42,
        bytes.fromhex("000102030405060708090a0b0c"),
    )
    assert derived.hex() == (
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    ), derived.hex()
    empty_salt = hkdf_sha256(bytes.fromhex("0b" * 22), b"", 42)
    assert empty_salt.hex() == (
        "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8"
    ), empty_salt.hex()


def test_hkdf_refuses_an_output_length_outside_one_block_schedule() -> None:
    for length in (0, 255 * 32 + 1):
        try:
            hkdf_sha256(ROOT_SEED, b"evidence", length, NESTED_KEY_SALT)
        except ValueError as error:
            assert "out of range" in str(error), error
        else:
            raise AssertionError(f"HKDF accepted a {length}-byte output length")


def test_nested_keys_are_deterministic_for_one_root() -> None:
    first = nested_signing_keys("runtime-2026-08", ROOT_SEED)
    second = nested_signing_keys("runtime-2026-08", ROOT_SEED)
    assert first == second
    assert tuple(first) == NESTED_KEY_PURPOSES
    for purpose, (key_id, seed) in first.items():
        assert key_id == f"runtime-2026-08.{purpose}"
        assert len(seed) == SEED_BYTES


def test_each_purpose_gets_its_own_signing_key() -> None:
    keys = nested_signing_keys("runtime-2026-08", ROOT_SEED)
    seeds = [seed for _, seed in keys.values()]
    assert len(set(seeds)) == len(NESTED_KEY_PURPOSES), keys
    assert ROOT_SEED not in seeds
    assert len({key_id for key_id, _ in keys.values()}) == len(NESTED_KEY_PURPOSES)


def test_a_different_root_seed_yields_entirely_different_nested_keys() -> None:
    first = nested_signing_keys("runtime-2026-08", ROOT_SEED)
    second = nested_signing_keys("runtime-2026-08", OTHER_SEED)
    assert {key_id for key_id, _ in first.values()} == {key_id for key_id, _ in second.values()}
    assert not {seed for _, seed in first.values()} & {seed for _, seed in second.values()}


def test_the_key_id_never_changes_a_derived_seed() -> None:
    named = nested_signing_keys("other-root-id", ROOT_SEED)
    original = nested_signing_keys("runtime-2026-08", ROOT_SEED)
    assert [seed for _, seed in named.values()] == [seed for _, seed in original.values()]
    assert [key_id for key_id, _ in named.values()] != [key_id for key_id, _ in original.values()]


def test_a_root_seed_that_is_not_thirty_two_bytes_is_refused() -> None:
    for seed in (b"", ROOT_SEED[:31], ROOT_SEED + b"\x00"):
        try:
            nested_signing_keys("runtime-2026-08", seed)
        except ValueError as error:
            assert "must be 32 bytes" in str(error), error
        else:
            raise AssertionError(f"a {len(seed)}-byte root signing seed was accepted")


if __name__ == "__main__":
    for name, test in sorted(globals().items()):
        if name.startswith("test_"):
            test()
    print("authority key derivation checks passed")
