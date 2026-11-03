#!/usr/bin/env python3
"""Publish and select an immutable Gent generation under host and store leases."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import shutil
import tempfile
from activate_install_support import (
    fail, file_digest, fsync_directory, fsync_file, generation, mutation_lease,
    reject_newer, release_parts, require_directory, require_executable,
    require_node_runtime, require_regular_file, selected_release, tree_digest,
)
LAUNCHER = """#!/usr/bin/env sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../lib/gent" && pwd)
target=$(readlink "$root/current")
case "$target" in releases/*) name=${target#releases/} ;; *) exit 1 ;; esac
case "$name" in ""|"."|".."|*/*) exit 1 ;; esac
exec "$root/releases/$name/$(basename -- "$0")" "$@"
"""

def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runtime_root", type=Path)
    parser.add_argument("release_name")
    for name in ("source-release", "source-supervisor", "source-auto-updater", "source-update-material", "bin-dir", "idle-data-dir"):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--stage-only", action="store_true")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--rollback-from")
    parser.add_argument("--expected-tree-sha256")
    return parser.parse_args()

def release_path(root: Path, name: str) -> Path:
    release_parts(name)
    path = root / "releases" / name
    require_directory(path)
    require_executable(path / "gent")
    require_executable(path / "gentd")
    return path

def copy_checked(source: Path, destination: Path, mode: int | None = None) -> None:
    before = file_digest(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    if mode is not None:
        destination.chmod(mode)
    if file_digest(source) != before or file_digest(destination) != before:
        fail("source changed while the private runtime stage was copied")
    fsync_file(destination)

def prepare_release(root: Path, source: Path, supervisor: Path | None,
                    updater: Path | None, material: Path | None) -> tuple[Path, str]:
    require_directory(source)
    for name in ("gent", "gentd"):
        require_executable(source / name)
    before = tree_digest(source)
    if (source / "runtime").exists():
        require_node_runtime(source)
    if (source / "authority").exists():
        for name in ("ordinary-authority.json", "root-keys.json"):
            require_regular_file(source / "authority" / name)
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    require_directory(root)
    stage = Path(tempfile.mkdtemp(prefix=".gent-stage-", dir=root))
    try:
        shutil.copytree(source, stage, dirs_exist_ok=True, copy_function=shutil.copy2)
        if tree_digest(source) != before or tree_digest(stage) != before:
            fail("source changed while the private runtime stage was copied")
        if supervisor is not None:
            copy_checked(supervisor, stage / "supervise-runtime-activation.py", 0o755)
            copy_checked(Path(__file__), stage / "activate-install.py", 0o755)
            copy_checked(Path(__file__).with_name("activate_install_support.py"), stage / "activate_install_support.py", 0o644)
        if updater is not None:
            copy_checked(updater, stage / "gent-auto-update.py", 0o700)
        if material is not None:
            require_directory(material)
            for name in ("runtime-release-trust.json", "runtime-release-cache.json"):
                copy_checked(material / name, stage / name, 0o600)
        fsync_directory(stage)
        return stage, tree_digest(stage)
    except BaseException:
        remove_private_stage(root, stage)
        raise

def remove_private_stage(root: Path, stage: Path) -> None:
    if stage.exists():
        if stage.parent.resolve() != root.resolve() or not stage.name.startswith(".gent-stage-") or stage.is_symlink():
            fail("private staging cleanup is outside the runtime root")
        shutil.rmtree(stage)

def publish_release(root: Path, requested: str, stage: Path, digest: str, lease) -> Path:
    lease.require()
    if tree_digest(stage) != digest:
        fail("prepared runtime identity changed before publication")
    current = selected_release(root)
    if current is not None and "+" in current and release_parts(current) == release_parts(requested):
        active = release_path(root, current)
        if tree_digest(active) == digest:
            return active
    releases = root / "releases"
    releases.mkdir(mode=0o700, exist_ok=True)
    require_directory(releases)
    destination = releases / generation(requested, digest)
    if destination.exists() or destination.is_symlink():
        fail("immutable runtime generation already exists")
    os.rename(stage, destination)
    fsync_directory(releases)
    return release_path(root, destination.name)

def select_release(root: Path, release: Path, digest: str, lease) -> None:
    lease.require()
    if tree_digest(release) != digest:
        fail("release identity changed before selection")
    selected_release(root)
    temporary = root / f".current-{os.getpid()}"
    if temporary.exists() or temporary.is_symlink():
        fail("a previous current-pointer staging file already exists")
    try:
        os.symlink(Path("releases") / release.name, temporary)
        os.replace(temporary, root / "current")
        fsync_directory(root)
    finally:
        temporary.unlink(missing_ok=True)
    if selected_release(root) != release.name or tree_digest(release) != digest:
        fail("selected runtime identity could not be verified")

def publish_launchers(bin_dir: Path, lease) -> None:
    lease.require()
    bin_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    require_directory(bin_dir)
    for name in ("gent", "gentd"):
        descriptor, temporary_name = tempfile.mkstemp(prefix=f".{name}-", dir=bin_dir)
        temporary = Path(temporary_name)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as output:
                output.write(LAUNCHER)
                output.flush()
                os.fsync(output.fileno())
            temporary.chmod(0o755)
            destination = bin_dir / name
            if destination.exists() or destination.is_symlink():
                require_regular_file(destination)
            os.replace(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)
    fsync_directory(bin_dir)

def refresh_updater(root: Path, release: Path, lease) -> None:
    lease.require()
    source = release / "gent-auto-update.py"
    if not source.exists():
        return
    temporary = root / f".gent-auto-update-{os.getpid()}"
    destination = root / "gent-auto-update.py"
    if destination.exists() or destination.is_symlink():
        require_regular_file(destination)
    try:
        copy_checked(source, temporary, 0o700)
        os.replace(temporary, destination)
        fsync_directory(root)
    finally:
        temporary.unlink(missing_ok=True)

def restore_selection(root: Path, previous: str | None, digest: str | None,
                      changed: str, lease) -> None:
    lease.require()
    if selected_release(root) != changed:
        fail("selection changed before rollback; no newer selection was overwritten")
    if previous is None:
        (root / "current").unlink()
        fsync_directory(root)
        return
    restored = release_path(root, previous)
    if tree_digest(restored) != digest:
        fail("prior runtime identity changed; rollback was refused")
    if "+" not in restored.name:
        stage, prepared = prepare_release(root, restored, None, None, None)
        try:
            restored = publish_release(root, restored.name, stage, prepared, lease)
        finally:
            remove_private_stage(root, stage)
    select_release(root, restored, digest, lease)
    refresh_updater(root, restored, lease)
    if selected_release(root) != restored.name or tree_digest(restored) != digest:
        fail("restored runtime selection could not be verified")

def activate_existing(args: argparse.Namespace) -> Path:
    with mutation_lease(args.runtime_root, args.idle_data_dir) as lease:
        current = selected_release(args.runtime_root)
        prior_digest = None
        if current:
            try:
                prior_digest = tree_digest(release_path(args.runtime_root, current))
            except (OSError, SystemExit):
                pass
        if args.rollback_from is not None:
            if current != args.rollback_from or not args.expected_tree_sha256:
                fail("rollback requires the exact still-selected generation and prior identity")
        else:
            reject_newer(args.runtime_root, args.release_name)
        source = release_path(args.runtime_root, args.release_name)
        digest = tree_digest(source)
        if args.expected_tree_sha256 and digest != args.expected_tree_sha256:
            fail("the requested release no longer matches its verified identity")
        if "+" not in source.name:
            stage, digest = prepare_release(args.runtime_root, source, None, None, None)
            try:
                source = publish_release(args.runtime_root, source.name, stage, digest, lease)
            finally:
                remove_private_stage(args.runtime_root, stage)
        try:
            select_release(args.runtime_root, source, digest, lease)
            refresh_updater(args.runtime_root, source, lease)
        except BaseException:
            if selected_release(args.runtime_root) == source.name and current != source.name:
                restore_selection(args.runtime_root, current, prior_digest, source.name, lease)
            raise
        return source

def install(args: argparse.Namespace) -> Path:
    if args.source_release is None or args.bin_dir is None:
        fail("source release and bin directory are required for installation")
    release_parts(args.release_name)
    reject_newer(args.runtime_root, args.release_name)
    stage, digest = prepare_release(args.runtime_root, args.source_release, args.source_supervisor,
                                    args.source_auto_updater, args.source_update_material)
    try:
        with mutation_lease(args.runtime_root, args.idle_data_dir) as lease:
            current = reject_newer(args.runtime_root, args.release_name)
            prior_digest = None
            if current:
                try:
                    prior_digest = tree_digest(release_path(args.runtime_root, current))
                except (OSError, SystemExit):
                    pass
            if current is not None and not args.force:
                fail(f"Gent is already installed in {args.bin_dir}; pass --force to replace it")
            release = publish_release(args.runtime_root, args.release_name, stage, digest, lease)
            if args.stage_only:
                return release
            publish_launchers(args.bin_dir, lease)
            try:
                select_release(args.runtime_root, release, digest, lease)
                refresh_updater(args.runtime_root, release, lease)
            except BaseException:
                if selected_release(args.runtime_root) == release.name and current != release.name:
                    restore_selection(args.runtime_root, current, prior_digest, release.name, lease)
                raise
            return release
    finally:
        remove_private_stage(args.runtime_root, stage)

def main() -> None:
    args = arguments()
    release = install(args) if args.source_release else activate_existing(args)
    if args.json:
        print(json.dumps({"release": release.name, "path": str(release.resolve()), "treeSha256": tree_digest(release)}))
    else:
        print(f"{'staged' if args.stage_only else 'activated'} {release.name}")

if __name__ == "__main__":
    try:
        main()
    except OSError as error:
        raise SystemExit(f"Gent activation failed: {error}") from error
