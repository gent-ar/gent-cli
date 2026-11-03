"""Finite Linux companion payload preparation; no signing or capability admission."""
import hashlib
import json
import os
import stat
from pathlib import Path

ROLES = ("gent-browser", "gent-canvas", "mate-canvas")

def relative(value):
    p = Path(value)
    if not value or len(value)>1024 or not value.isascii() or "\\" in value or p.is_absolute() or any(x in ("", ".", "..") for x in value.split("/")):
        raise ValueError("resource path must be a finite canonical relative path")
    return p

def checked(root, value):
    p = root / relative(value)
    if any(x.is_symlink() for x in (p, *p.parents)) or p.resolve(strict=True) != p:
        raise ValueError("resource alias rejected")
    return p

def stamp(s):
    return s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns

def regular_file(path):
    # NONBLOCK prevents a replacement FIFO/device from stalling preflight.
    fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC)
    file=os.fdopen(fd,"rb")
    if not stat.S_ISREG(os.fstat(file.fileno()).st_mode):
        file.close()
        raise ValueError("regular file required")
    return file

def pin(root, value, maximum):
    p = checked(root, value)
    with regular_file(p) as f:
        before = os.fstat(f.fileno())
        if not p.is_file() or not 64 <= before.st_size <= maximum:
            raise ValueError("resource file bounds")
        h = hashlib.sha256()
        total = 0
        while chunk := f.read(65536):
            total += len(chunk)
            if total > maximum:
                raise ValueError("resource grew")
            h.update(chunk)
        if stamp(before) != stamp(os.fstat(f.fileno())) or stamp(before) != stamp(p.stat()) or total != before.st_size:
            raise ValueError("resource generation changed while hashing")
    return {"path": value, "sha256": h.hexdigest(), "bytes": total}

def bounded_read(path, maximum):
    with regular_file(path) as file:
        before = os.fstat(file.fileno())
        if before.st_size > maximum: raise ValueError("metadata size bound")
        value = file.read(maximum + 1)
        if len(value) > maximum or stamp(before) != stamp(os.fstat(file.fileno())) or stamp(before) != stamp(path.stat()):
            raise ValueError("metadata generation or size changed")
    return value

def network_manifest(root, value):
    network = checked(root, value)
    manifest = checked(root, value + "/manifest.json")
    if manifest.stat().st_size > 131072:
        raise ValueError("network manifest bounds")
    raw = bounded_read(manifest, 131072)
    data = json.loads(raw)
    if set(data) != {"schema", "files"} or data["schema"] != 1 or not 1 <= len(data["files"]) <= 512:
        raise ValueError("network manifest shape")
    names = set()
    total = 0
    for entry in data["files"]:
        if set(entry) != {"path", "sha256"} or entry["path"] in names:
            raise ValueError("network entry shape or duplicate")
        relative(entry["path"])
        path = checked(network, entry["path"])
        # Network closure permits tiny data files; all executable bounds remain finite.
        with regular_file(path) as f:
            before = os.fstat(f.fileno())
            if not path.is_file() or before.st_size > 16 << 20:
                raise ValueError("network file bounds")
            digest = hashlib.sha256()
            count = 0
            while chunk := f.read(65536):
                count += len(chunk)
                if count > 16 << 20: raise ValueError("network grew")
                digest.update(chunk)
            if stamp(before) != stamp(os.fstat(f.fileno())) or stamp(before) != stamp(path.stat()) or count != before.st_size:
                raise ValueError("network generation changed")
        if digest.hexdigest() != entry["sha256"]: raise ValueError("network digest mismatch")
        total += count
        if total > 64 << 20: raise ValueError("network aggregate bounds")
        names.add(entry["path"])
    observed = set()
    count = 0
    for directory, dirs, files in os.walk(network, followlinks=False):
        for name in dirs + files:
            path = Path(directory)/name
            count += 1
            if count > 1024 or path.is_symlink(): raise ValueError("network tree alias or bounds")
        for name in files:
            value = (Path(directory)/name).relative_to(network).as_posix()
            if value != "manifest.json": observed.add(value)
    required = {"usr/bin/slirp4netns", "usr/sbin/iptables", "usr/sbin/ip6tables", "usr/sbin/iptables-restore", "usr/sbin/ip6tables-restore"}
    if observed != names or not required <= names: raise ValueError("network closure incomplete")
    if bounded_read(manifest, 131072) != raw: raise ValueError("network manifest changed")
    return hashlib.sha256(raw).hexdigest()

def prepare(root, spec, ordinary_digest, expires):
    root = Path(root)
    if not root.is_absolute() or root.resolve(strict=True) != root:
        raise ValueError("release root must be canonical")
    if set(spec) != {"platform", "helper", "node", "gent", "network", "mcpEntries"} or set(spec["mcpEntries"]) != set(ROLES):
        raise ValueError("complete exact resource set required")
    if spec["platform"] not in ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu") or spec["node"] != "runtime/node/bin/node":
        raise ValueError("packaged platform/Node layout mismatch")
    if len(ordinary_digest) != 64 or any(c not in "0123456789abcdef" for c in ordinary_digest):
        raise ValueError("ordinary authority digest malformed")
    files = [pin(root, spec[k], maximum) for k, maximum in (("helper",64<<20),("node",512<<20),("gent",128<<20))]
    entries = {role: pin(root, spec["mcpEntries"][role], 2<<20) for role in ROLES}
    if len({x["path"] for x in [*files,*entries.values()]}) != 6:
        raise ValueError("duplicate resource paths")
    network_digest = network_manifest(root, spec["network"])
    return {"schema":1,"authorityReleaseSha256":ordinary_digest,"expiresAtUnixSeconds":expires,"revoked":False,
            "platform":spec["platform"],"mxcRevision":"fda29ac63bc306b7f97f3b86b4b63d44f7f40cf2",
            "baselinePolicy":"mxc-1.0.1-native-readonly","cpuPolicy":"monitored-total-5ms",
            "helper":files[0],"node":files[1],"gent":files[2],
            "network":{"path":spec["network"],"manifestSha256":network_digest},"mcpEntries":entries}

def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
