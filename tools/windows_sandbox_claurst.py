"""Finite paired Claurst release inputs; no installation or acceptance claim."""
import json
import os
import re
from pathlib import Path, PurePosixPath

from windows_sandbox_package import finite_file, no_redirect, sha, unique_object


def digest(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value)


def relative(value):
    if (not isinstance(value, str) or not value or '\\' in value or ':' in value or '\0' in value
            or PurePosixPath(value).is_absolute() or any(p in ('', '.', '..') for p in value.split('/'))):
        raise ValueError('unsafe Claurst release resource path')
    return value


def physical(root, raw):
    path = root / relative(raw)
    return finite_file(root, path)


def collect_claurst(windows, provider, helper_pin, selected_root=None):
    allowed = {'provider', 'platform', 'executableSha256', 'credentialContract', 'files', 'osLibraries'}
    if (set(provider) != allowed or provider.get('platform') != 'win32-x64'
            or provider.get('credentialContract') != 'claurst-daemon-model-mcp-v1'
            or not digest(provider.get('executableSha256'))):
        raise ValueError('Claurst release requires its exact paired model/MCP resource contract')
    rows = provider['files']
    kinds = {'package', 'gitRuntime', 'powerShellRuntime', 'powerShellLauncher', 'osGlobalizationData'}
    if not 1 <= len(rows) <= 1024 or any(set(row) != {'root', 'path', 'sha256'} or not digest(row['sha256']) or row['root'] not in kinds for row in rows):
        raise ValueError('invalid finite Claurst release resources')
    root = Path(selected_root) if selected_root is not None else windows.parent / 'claurst'
    no_redirect(root)
    if not root.is_dir() or root.is_symlink() or getattr(root.lstat(), 'st_file_attributes', 0) & 0x400:
        raise ValueError('Claurst paired runtime directory is unavailable')
    files, expected = [], {}

    def add(raw, wanted=None):
        path = physical(root, raw)
        value = sha(path)
        if wanted is not None and value != wanted:
            raise ValueError('Claurst release artifact differs from reviewed digest: ' + raw)
        target = 'runtime/claurst/' + raw
        if any(old.casefold() == target.casefold() for old in expected):
            raise ValueError('duplicate Claurst release resource')
        files.append((path, target))
        expected[target] = value

    package = [row for row in rows if row['root'] == 'package']
    image = [row for row in package if row['path'] == 'claurst.exe']
    if len(image) != 1 or image[0]['sha256'] != provider['executableSha256']:
        raise ValueError('Claurst release SDK image is not bound to its resource contract')
    for row in package:
        add(relative(row['path']), row['sha256'])
    manifest_path = physical(root, 'model-server/model-runtime.json')
    if manifest_path.stat().st_size > 65536:
        raise ValueError('Claurst model manifest exceeds its bound')
    model = json.loads(manifest_path.read_text(encoding='utf-8'), object_pairs_hook=unique_object)
    if (not isinstance(model, dict) or set(model) != {'schema', 'transport', 'helperSha256', 'workerSha256', 'osLibraries'}
            or type(model['schema']) is not int or model['schema'] != 1
            or model['transport'] != 'server-context-stdio-v1' or model['helperSha256'] != helper_pin
            or not digest(model['workerSha256'])):
        raise ValueError('Claurst model manifest does not match the paired helper')
    libraries = model['osLibraries']
    names = []
    if not isinstance(libraries, list) or not 1 <= len(libraries) <= 128:
        raise ValueError('invalid finite Claurst model OS dependencies')
    for row in libraries:
        if (not isinstance(row, dict) or set(row) != {'name', 'sha256'} or not digest(row.get('sha256'))
                or not isinstance(row.get('name'), str) or not re.fullmatch('[A-Za-z0-9_][A-Za-z0-9_.-]{0,123}\.dll', row['name'], re.IGNORECASE)
                or row['name'].casefold() in names):
            raise ValueError('invalid finite Claurst model OS dependency')
        names.append(row['name'].casefold())
    add('model-server/model-runtime.json')
    add('model-server/gent-contained-model-factory.exe', model['workerSha256'])
    llama = root / 'llama'
    if not llama.is_dir() or not (llama / 'llama-server.exe').is_file():
        raise ValueError('Claurst reviewed llama runtime payload is unavailable')
    payload = []
    no_redirect(llama)
    for folder, directories, names in os.walk(llama, followlinks=False):
        for name in directories:
            no_redirect(Path(folder) / name)
        for name in names:
            payload.append(Path(folder) / name)
    payload.sort()
    if not 1 <= len(payload) <= 512:
        raise ValueError('Claurst llama runtime payload exceeds its finite file bound')
    for path in payload:
        if path.suffix.casefold() in ('.gguf', '.ggml'):
            raise ValueError('Claurst model weights require separate managed provisioning')
        add(path.relative_to(root).as_posix())
    return files, expected
