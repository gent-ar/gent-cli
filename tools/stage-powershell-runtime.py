import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import stat
import struct
import zipfile


REQUIRED = {'pwsh.exe', 'pwsh.dll', 'pwsh.deps.json', 'pwsh.runtimeconfig.json',
            'hostfxr.dll', 'hostpolicy.dll', 'coreclr.dll', 'System.Private.CoreLib.dll',
            'System.Management.Automation.dll'}


def sha(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def relative(name):
    path = PurePosixPath(name)
    reserved = {'con', 'prn', 'aux', 'nul'} | {f'{kind}{n}' for kind in ('com', 'lpt') for n in range(1, 10)}
    if (not name or len(name) > 256 or chr(92) in name or path.is_absolute()
            or any(not p or p in ('.', '..') or p.endswith('.')
                   or p.split('.')[0].lower() in reserved
                   or not re.fullmatch(r'[A-Za-z0-9_.-]+', p) for p in name.split('/'))):
        raise ValueError('unsafe PowerShell runtime archive path')
    return path


def stage(archive, expected, version, output):
    archive, output = Path(archive), Path(output)
    if (not re.fullmatch('[0-9a-f]{64}', expected) or not re.fullmatch(r'7\.\d+\.\d+', version)
            or archive.is_symlink() or not archive.is_file()
            or getattr(archive.stat(), 'st_file_attributes', 0) & 0x400
            or sha(archive) != expected):
        raise ValueError('PowerShell archive differs from its reviewed identity')
    if output.exists():
        raise ValueError('PowerShell runtime output must be new')
    proof_path = output.with_name(output.name + '.manifest.json')
    if proof_path.exists():
        raise ValueError('PowerShell runtime manifest must be new')
    with zipfile.ZipFile(archive) as package:
        files, names = [], set()
        for entry in package.infolist():
            if entry.orig_filename != entry.filename:
                raise ValueError('normalized PowerShell archive entry is not admitted')
            if entry.is_dir():
                relative(entry.filename.rstrip('/'))
                continue
            path = relative(entry.filename)
            if (path.as_posix().lower() in names or entry.flag_bits & 1
                    or stat.S_ISLNK(entry.external_attr >> 16)):
                raise ValueError('ambiguous or redirected PowerShell archive entry')
            names.add(path.as_posix().lower())
            files.append((entry, path))
        if (not REQUIRED.issubset({str(p) for _, p in files}) or len(files) > 1024
                or sum(e.file_size for e, _ in files) > 1024 ** 3):
            raise ValueError('PowerShell runtime closure is incomplete or exceeds its bound')
        with package.open('pwsh.exe') as image:
            header = image.read(4096)
            pe = struct.unpack_from('<I', header, 60)[0] if header[:2] == b'MZ' else 4096
            if pe + 6 > len(header) or header[pe:pe+6] != b'PE\0\0d\x86':
                raise ValueError('PowerShell runtime must contain an x64 PE executable')
        output.mkdir(parents=True)
        records = []
        for entry, relative_path in sorted(files, key=lambda row: str(row[1])):
            destination = output / relative_path
            destination.parent.mkdir(parents=True, exist_ok=True)
            digest, copied = hashlib.sha256(), 0
            with package.open(entry) as source, destination.open('xb') as target:
                while chunk := source.read(1024 * 1024):
                    target.write(chunk)
                    digest.update(chunk)
                    copied += len(chunk)
            if copied != entry.file_size:
                raise ValueError('PowerShell archive extraction was incomplete')
            records.append({'root': 'powerShellRuntime', 'path': str(relative_path),
                            'sha256': digest.hexdigest(), 'bytes': copied})
    if sha(archive) != expected:
        raise ValueError('PowerShell archive changed during staging')
    proof = {'schemaVersion': 1, 'version': version, 'archiveSha256': expected,
             'runtime': str(output.resolve()), 'files': records,
             'fileCount': len(records), 'installed': False, 'psecAccepted': False}
    proof_path.write_text(json.dumps(proof, indent=2) + '\n', encoding='utf-8')
    return proof


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    proof = stage(args.archive, args.sha256, args.version, args.out)
    print(json.dumps({key: proof[key] for key in ['runtime', 'fileCount', 'installed', 'psecAccepted']}))


if __name__ == '__main__':
    main()
