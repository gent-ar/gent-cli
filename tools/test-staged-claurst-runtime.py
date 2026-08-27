#!/usr/bin/env python3
import argparse
import os
import subprocess
from pathlib import Path


def arguments():
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime-dir", type=Path, required=True)
    return parser.parse_args()


def executable(path):
    if not path.is_file():
        raise ValueError(f"missing runtime executable: {path}")
    if os.name != "nt" and not path.stat().st_mode & 0o111:
        raise ValueError(f"runtime executable is not executable: {path}")
    return path


def run(path, argument):
    result = subprocess.run(
        [str(path), argument],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=30,
    )
    if result.returncode != 0:
        raise RuntimeError(f"{path.name} {argument} failed with exit code {result.returncode}")


def main():
    args = arguments()
    suffix = ".exe" if os.name == "nt" else ""
    claurst = executable(args.runtime_dir / f"claurst{suffix}")
    llama = executable(args.runtime_dir / "llama" / f"llama-server{suffix}")
    run(claurst, "--help")
    run(llama, "--version")
    print("Staged Claurst and llama.cpp runtime startup checks passed")


if __name__ == "__main__":
    main()
