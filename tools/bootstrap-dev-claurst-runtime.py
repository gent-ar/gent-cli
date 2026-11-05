import argparse
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def arguments():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target")
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--out-dir", type=Path)
    parser.add_argument("--source-dir", type=Path)
    return parser.parse_args()


def host_target():
    output = subprocess.run(
        ["rustc", "-vV"], check=True, capture_output=True, text=True
    ).stdout
    for line in output.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ")
    raise RuntimeError("rustc did not report a host target")


def main():
    args = arguments()
    command = [
        sys.executable,
        str(ROOT / "tools/stage-claurst-runtime.py"),
        "--target",
        args.target or host_target(),
        "--out-dir",
        str(args.out_dir or ROOT / "target" / args.profile / "runtime/claurst"),
    ]
    if args.source_dir is not None:
        command.extend(["--source-dir", str(args.source_dir)])
    subprocess.run(command, check=True)


if __name__ == "__main__":
    main()
