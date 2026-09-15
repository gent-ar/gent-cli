#!/usr/bin/env python3
import argparse
import hashlib
import json
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path


PINS = Path(__file__).resolve().parents[1] / "fixtures" / "provider-contracts" / "pins.json"


def pinned_artifacts(path=PINS):
    pins = json.loads(path.read_text(encoding="utf-8"))
    claurst = pins["providers"]["claurst"]["artifacts"]
    llama = pins["runtimes"]["llama_cpp"]["artifacts"]
    if set(claurst) != set(llama):
        raise ValueError("Claurst and llama.cpp pins must cover the same targets")
    return {
        target: {
            "claurst": (claurst[target]["url"], claurst[target]["sha256"], claurst[target]["binary"]),
            "llama": (llama[target]["url"], llama[target]["sha256"], llama[target]["binary"]),
        }
        for target in claurst
    }


ARTIFACTS = pinned_artifacts()


def arguments():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True, choices=ARTIFACTS)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--source-dir", type=Path)
    parser.add_argument("--claurst-source-dir", type=Path)
    return parser.parse_args()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def member_name(archive, name):
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as bundle:
            names = [entry.filename for entry in bundle.infolist() if not entry.is_dir() and Path(entry.filename).name == name]
            if len(names) != 1:
                raise ValueError(f"archive does not contain exactly one {name}")
            return names[0]
    with tarfile.open(archive, "r:gz") as bundle:
        names = [entry.name for entry in bundle.getmembers() if entry.isfile() and Path(entry.name).name == name]
        if len(names) != 1:
            raise ValueError(f"archive does not contain exactly one {name}")
        return names[0]


def extract(archive, name, destination):
    member = member_name(archive, name)
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as bundle:
            with bundle.open(member) as source, destination.open("wb") as output:
                shutil.copyfileobj(source, output)
    else:
        with tarfile.open(archive, "r:gz") as bundle:
            source = bundle.extractfile(member)
            if source is None:
                raise ValueError(f"archive cannot read {name}")
            with source, destination.open("wb") as output:
                shutil.copyfileobj(source, output)


def llama_members(archive):
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as bundle:
            members = [entry.filename for entry in bundle.infolist() if not entry.is_dir()]
    else:
        with tarfile.open(archive, "r:gz") as bundle:
            members = [entry.name for entry in bundle.getmembers() if entry.isfile() or entry.issym()]
    names = [Path(member).name for member in members]
    if not names or len(names) != len(set(names)):
        raise ValueError("llama runtime archive has unsafe member names")
    return members


def extract_llama_runtime(archive, destination):
    destination.mkdir(parents=True, exist_ok=True)
    members = llama_members(archive)
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as bundle:
            for member in members:
                with bundle.open(member) as source, (destination / Path(member).name).open("wb") as output:
                    shutil.copyfileobj(source, output)
    else:
        with tarfile.open(archive, "r:gz") as bundle:
            for member in members:
                source = bundle.extractfile(member)
                if source is None:
                    raise ValueError(f"archive cannot read {member}")
                with source, (destination / Path(member).name).open("wb") as output:
                    shutil.copyfileobj(source, output)
    server_name = next(
        (name for name in ("llama-server", "llama-server.exe") if (destination / name).is_file()),
        None,
    )
    if server_name is None:
        raise ValueError("llama runtime archive does not contain llama-server")
    server = destination / server_name
    for path in destination.iterdir():
        path.chmod(0o755)
    return server


def source_for(directory, url, expected):
    if directory is not None:
        candidate = directory / Path(url).name
        if candidate.is_file():
            if digest(candidate) != expected:
                raise ValueError(f"artifact digest does not match: {candidate}")
            return candidate
    temporary = tempfile.NamedTemporaryFile(delete=False)
    temporary.close()
    path = Path(temporary.name)
    try:
        with urllib.request.urlopen(url) as source, path.open("wb") as output:
            shutil.copyfileobj(source, output)
        if digest(path) != expected:
            raise ValueError(f"artifact digest does not match: {url}")
        return path
    except BaseException:
        path.unlink(missing_ok=True)
        raise


def stage_claurst_source(source, destination, profile, target):
    manifest = source / "src-rust" / "Cargo.toml"
    if not manifest.is_file():
        raise ValueError(f"Claurst source does not contain {manifest}")
    command = [
        "cargo",
        "build",
        "--manifest-path",
        str(manifest),
        "--bin",
        "claurst",
        "--target",
        target,
    ]
    if profile == "release":
        command.append("--release")
    subprocess.run(command, check=True)
    binary_name = "claurst.exe" if destination.suffix == ".exe" else "claurst"
    binary = source / "src-rust" / "target" / target / profile / binary_name
    if not binary.is_file():
        raise ValueError(f"Claurst source did not build {binary}")
    shutil.copy2(binary, destination)
    destination.chmod(0o755)


def main():
    args = arguments()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    staged = []
    try:
        for kind, (url, expected, name) in ARTIFACTS[args.target].items():
            if kind == "claurst" and args.claurst_source_dir is not None:
                stage_claurst_source(
                    args.claurst_source_dir, args.out_dir / name, args.profile, args.target
                )
                continue
            archive = source_for(args.source_dir, url, expected)
            staged.append(archive)
            if kind == "llama":
                extract_llama_runtime(archive, args.out_dir / "llama")
            else:
                extract(archive, name, args.out_dir / name)
                (args.out_dir / name).chmod(0o755)
    finally:
        for archive in staged:
            if args.source_dir is None or archive.parent != args.source_dir:
                archive.unlink(missing_ok=True)
    print(json.dumps({"target": args.target, "directory": str(args.out_dir), "files": sorted(path.name for path in args.out_dir.iterdir())}))


if __name__ == "__main__":
    main()
