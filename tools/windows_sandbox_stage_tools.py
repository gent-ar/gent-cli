#!/usr/bin/env python3
import argparse
import hashlib
import json
import shutil
from pathlib import Path, PurePosixPath

TOOL_LIST = Path(__file__).resolve().parent.parent / 'crates/gentd/windows-command-tools.json'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def command_tool_paths():
    closure = json.loads(TOOL_LIST.read_text(encoding='utf-8'))
    binaries = ['usr/bin/' + name for name in closure['tools'] + closure['libraries'] + closure['gentTools']]
    return binaries + closure['licenses']


def stage(distribution, build, runtime):
    git = runtime / 'git'
    rows = []
    aliases = json.loads(TOOL_LIST.read_text(encoding='utf-8'))['privateAliases']
    for relative in command_tool_paths():
        target = git / relative
        name = PurePosixPath(relative).name
        built = build / name
        if name in aliases:
            shutil.copyfile(git / 'usr/bin' / aliases[name], target)
        elif not target.is_file() or built.is_file():
            source = built if built.is_file() else distribution / relative
            if not source.is_file():
                raise ValueError('pinned Git distribution is missing ' + relative)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
        rows.append({'root': 'gitRuntime', 'path': relative, 'sha256': sha(target)})
    manifest_path = runtime / 'provider-resources.json'
    manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
    for provider in manifest['providers']:
        known = {f['path'].casefold(): f for f in provider['files'] if f['root'] == 'gitRuntime'}
        for row in rows:
            existing = known.get(row['path'].casefold())
            if existing is None:
                provider['files'].append(dict(row))
            elif existing['sha256'] != row['sha256']:
                raise ValueError('command tool digest conflicts with the finite contract: ' + row['path'])
    manifest_path.write_text(json.dumps(manifest), encoding='utf-8')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--git-distribution', required=True, type=Path)
    parser.add_argument('--native-build', required=True, type=Path)
    parser.add_argument('--runtime', required=True, type=Path)
    arguments = parser.parse_args()
    stage(arguments.git_distribution, arguments.native_build, arguments.runtime)
