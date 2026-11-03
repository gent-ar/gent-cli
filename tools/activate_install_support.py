"""Filesystem identity and leases shared by the signed Unix installers."""
from __future__ import annotations
from contextlib import contextmanager
import hashlib
import os
from pathlib import Path
import re
import stat
import struct
import uuid
try:
    import fcntl
except ImportError:
    fcntl = None

def fail(message: str) -> None:
    raise SystemExit(f"Gent activation refused: {message}")

def lstat(path: Path):
    try:
        return path.lstat()
    except FileNotFoundError:
        fail(f"missing {path}")

def require_directory(path: Path) -> None:
    if not stat.S_ISDIR(lstat(path).st_mode) or path.is_symlink():
        fail(f"{path} must be a real directory")

def require_regular_file(path: Path) -> None:
    if not stat.S_ISREG(lstat(path).st_mode) or path.is_symlink():
        fail(f"{path} must be a real file")

def require_executable(path: Path) -> None:
    require_regular_file(path)
    if lstat(path).st_mode & 0o111 == 0:
        fail(f"{path} is not executable")

def require_node_runtime(release: Path) -> None:
    for relative in ("runtime/node/bin/node", "runtime/node/bin/npm",
                     "runtime/node/lib/node_modules/npm/bin/npm-cli.js",
                     "runtime/claurst/claurst", "runtime/claurst/llama/llama-server"):
        require_regular_file(release / relative)

def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)

def fsync_file(path: Path) -> None:
    with path.open("rb") as source:
        os.fsync(source.fileno())

def file_digest(path: Path) -> bytes:
    require_regular_file(path)
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            digest.update(block)
    return digest.digest()

def tree_digest(root: Path) -> str:
    require_directory(root)
    entries, total = [], 0
    for path in root.rglob("*"):
        mode = lstat(path).st_mode
        if path.is_symlink() or not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):
            fail(f"{path} must be a real file or directory")
        if getattr(lstat(path), "st_file_attributes", 0) & 0x400:
            fail(f"{path} cannot be a reparse point")
        if os.name != "nt" and mode & 0o7000:
            fail(f"{path} has special permission bits")
        if stat.S_ISREG(mode):
            total += path.stat().st_size
            entries.append((path.relative_to(root).as_posix().encode("utf-8"), path, mode))
        if len(entries) > 20000 or total > 8 * 1024 ** 3:
            fail("runtime tree exceeds the file-count or byte limit")
    digest = hashlib.sha256(b"gent-runtime-tree-v1\0")
    for relative, path, mode in sorted(entries):
        digest.update(struct.pack(">Q", len(relative)))
        digest.update(relative)
        digest.update(file_digest(path))
        digest.update(struct.pack(">I", mode & 0o777 if os.name != "nt" else 0))
    return digest.hexdigest()

def release_parts(name: str) -> tuple[str, str]:
    if Path(name).name != name or "\\" in name:
        fail("release name is not a single path component")
    match = re.fullmatch(r"(v[0-9]+(?:\.[0-9]+){0,2})(?:\+[0-9a-f]{16}\.(?:[0-9a-f]{8}|[0-9a-f]{64}))?-(.+)", name)
    if match is None:
        fail("release identity has no valid version and target")
    return match.group(1), match.group(2)

def version_of(name: str) -> tuple[int, ...]:
    return tuple(map(int, release_parts(name)[0][1:].split(".")))

def generation(name: str, digest: str) -> str:
    version, target = release_parts(name)
    return f"{version}+{digest[:16]}.{uuid.uuid4().hex[:8]}-{target}"

def selected_release(root: Path) -> str | None:
    current = root / "current"
    try:
        details = current.lstat()
    except FileNotFoundError:
        return None
    if not stat.S_ISLNK(details.st_mode):
        fail("current exists but is not a symlink")
    target = Path(os.readlink(current))
    if target.parent != Path("releases") or target.name in {"", ".", ".."}:
        fail("current does not point to a managed release")
    release_parts(target.name)
    return target.name

def reject_newer(root: Path, requested: str) -> str | None:
    current = selected_release(root)
    if current is not None and version_of(current) > version_of(requested):
        fail("a newer runtime is already selected; no files were published")
    return current

class MutationLease:
    def __init__(self):
        self.held = True
    def require(self):
        if not self.held:
            fail("runtime mutation lease is not held")

@contextmanager
def file_lease(directory: Path, name: str, busy: str):
    if fcntl is None:
        fail("this activation helper requires POSIX file leases")
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    require_directory(directory)
    path = directory / name
    if path.exists() or path.is_symlink():
        require_regular_file(path)
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "a+b") as lock:
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise SystemExit(f"Gent activation refused: {busy}") from error
        try:
            yield
        finally:
            fcntl.flock(lock.fileno(), fcntl.LOCK_UN)

@contextmanager
def mutation_lease(root: Path, data: Path | None):
    if data is None and selected_release(root) is not None:
        fail("updating an existing runtime requires explicit --idle-data-dir")
    @contextmanager
    def no_data():
        yield
    data_lease = no_data() if data is None else file_lease(data, "gentd.lock", f"gentd is running for {data}; stop it before updating")
    with data_lease:
        with file_lease(root, ".install.lock", "another Gent install is already in progress"):
            lease = MutationLease()
            try:
                yield lease
            finally:
                lease.held = False
