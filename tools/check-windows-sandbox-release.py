"""Fail a Windows release that omits its contained provider runtime."""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile


def check(dist, version, target):
    manifests = []
    for path in Path(dist).glob('*.manifest.json'):
        value = json.loads(path.read_text(encoding='utf-8'))
        if value.get('version') == version and value.get('target') == target:
            manifests.append(value)
    if len(manifests) != 1:
        raise ValueError('expected one Windows release manifest for the selected version and target')
    manifest = manifests[0]
    if 'runtime/windows' not in manifest.get('runtimes', []):
        raise ValueError('Windows release omits the required sandbox runtime')
    archive = manifest.get('archive', {}).get('name', '')
    if not isinstance(archive, str) or Path(archive).name != archive or not archive.endswith('.zip'):
        raise ValueError('invalid Windows release archive name')
    required = {
        'runtime/windows/provider-helper.exe',
        'runtime/windows/provider-resources.json',
        'runtime/windows/reviewed-external-proxy-policy.json',
        'runtime/windows/proxy-package.msix',
        'runtime/windows/git/etc/fstab',
        'runtime/windows/git/usr/bin/bash.exe',
        'runtime/windows/git/usr/bin/msys-2.0.dll',
        'runtime/claurst/model-server/model-runtime.json',
        'runtime/claurst/model-server/gent-contained-model-factory.exe',
    }
    with zipfile.ZipFile(Path(dist) / archive) as package:
        names = package.namelist()
        if len(names) != len(set(names)):
            raise ValueError('Windows release archive contains duplicate entries')
        for suffix in required:
            if sum(name.endswith('/' + suffix) for name in names) != 1:
                raise ValueError('Windows release is missing or duplicates ' + suffix)
        def read(suffix):
            return package.read(next(name for name in names if name.endswith('/' + suffix)))
        from windows_sandbox_package import reviewed_providers, unique_object
        policy = json.loads(read('runtime/windows/reviewed-external-proxy-policy.json'), object_pairs_hook=unique_object)
        if set(reviewed_providers(policy)) != {'codex', 'claude', 'claurst'}:
            raise ValueError('Windows release sandbox does not cover all three providers')
        helper = hashlib.sha256(read('runtime/windows/provider-helper.exe')).hexdigest()
        resources = hashlib.sha256(read('runtime/windows/provider-resources.json')).hexdigest()
        if policy.get('helperSha256') != helper or policy.get('resourcesSha256') != resources:
            raise ValueError('Windows release sandbox descriptor does not match its archived helper or resources')
        fstab = hashlib.sha256(read('runtime/windows/git/etc/fstab')).hexdigest()
        if fstab != 'af2184544d99dd656f4e5e119d80f2cb15f03c29d3d8467da1df5aa330508f4a':
            raise ValueError('Windows release Git mount configuration is not reviewed')
        model = json.loads(read('runtime/claurst/model-server/model-runtime.json'), object_pairs_hook=unique_object)
        worker = hashlib.sha256(read('runtime/claurst/model-server/gent-contained-model-factory.exe')).hexdigest()
        if model.get('helperSha256') != helper or model.get('workerSha256') != worker:
            raise ValueError('Windows release model runtime is not bound to the archived helper and worker')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dist', type=Path, required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--target', required=True)
    args = parser.parse_args()
    check(args.dist, args.version, args.target)


if __name__ == '__main__':
    main()
