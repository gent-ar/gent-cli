#!/usr/bin/env python3
import argparse
import shutil
from pathlib import Path


def arguments():
    parser = argparse.ArgumentParser()
    parser.add_argument("--node", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--windows", action="store_true")
    return parser.parse_args()


def main():
    args = arguments()
    node = args.node
    if not node.is_file():
        resolved = shutil.which(str(node))
        if resolved is None:
            raise SystemExit(f"Node executable was not found: {args.node}")
        node = Path(resolved)
    node = node.resolve()
    source_bin = node.parent
    source_root = source_bin if args.windows else source_bin.parent
    npm_name = "npm.cmd" if args.windows else "npm"
    npm = source_bin / npm_name
    npm_root = source_root / "node_modules" / "npm" if args.windows else source_root / "lib" / "node_modules" / "npm"
    required = [node, npm, npm_root / "bin" / "npm-cli.js"]
    if any(not path.is_file() for path in required):
        raise SystemExit("Node runtime is incomplete")
    destination_bin = args.out_dir / "bin"
    destination_npm = args.out_dir / "lib" / "node_modules" / "npm"
    destination_bin.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(node, destination_bin / node.name)
    shutil.copyfile(npm.resolve(), destination_bin / npm_name)
    shutil.copytree(npm_root, destination_npm, symlinks=False)
    for path in (destination_bin / node.name, destination_bin / npm_name):
        path.chmod(0o755)


if __name__ == "__main__":
    main()
