"""Require a sandbox Claurst runtime to contain the exact pinned release payload."""
import argparse
from pathlib import Path

from windows_sandbox_package import finite_file, no_redirect, sha


def verify(official, sandbox):
    official, sandbox = Path(official), Path(sandbox)
    no_redirect(official)
    no_redirect(sandbox)
    files = [path for path in official.rglob('*') if path.is_file() or path.is_symlink()]
    if not files:
        raise ValueError('official Claurst release runtime is empty')
    official_llama = official / 'llama'
    sandbox_llama = sandbox / 'llama'
    no_redirect(official_llama)
    no_redirect(sandbox_llama)
    def members(root):
        return {path.relative_to(root).as_posix() for path in root.rglob('*')
                if path.is_file() or path.is_symlink()}
    if members(official_llama) != members(sandbox_llama):
        raise ValueError('sandbox llama file set differs from the pinned release')
    for source in files:
        relative = source.relative_to(official)
        checked_source = finite_file(official, source)
        candidate = finite_file(sandbox, sandbox / relative)
        if sha(checked_source) != sha(candidate):
            raise ValueError('sandbox Claurst differs from the pinned release: ' + relative.as_posix())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--official', type=Path, required=True)
    parser.add_argument('--sandbox', type=Path, required=True)
    args = parser.parse_args()
    verify(args.official, args.sandbox)


if __name__ == '__main__':
    main()
