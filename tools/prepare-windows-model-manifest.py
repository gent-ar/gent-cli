"""Prepare a model manifest bound to independently pinned release inputs."""
import argparse
import json
from pathlib import Path
import re

from windows_sandbox_package import ordinary, sha, unique_object
from windows_sandbox_claurst import digest


def prepare(template, template_sha256, helper, helper_sha256, worker, worker_sha256, output):
    template, helper, worker = (ordinary(Path(p)) for p in (template, helper, worker))
    for path, expected in ((template, template_sha256), (helper, helper_sha256),
                           (worker, worker_sha256)):
        if not digest(expected) or sha(path) != expected:
            raise ValueError('model manifest input differs from its reviewed digest')
    if template.stat().st_size > 65536:
        raise ValueError('model manifest template exceeds its bound')
    model = json.loads(template.read_text(encoding='utf-8'), object_pairs_hook=unique_object)
    if (not isinstance(model, dict)
            or set(model) != {'schema', 'transport', 'helperSha256', 'workerSha256', 'osLibraries'}
            or type(model['schema']) is not int or model['schema'] != 1
            or model['transport'] != 'server-context-stdio-v1'
            or model['workerSha256'] != worker_sha256
            or not isinstance(model['osLibraries'], list) or not model['osLibraries']):
        raise ValueError('model manifest template does not match the reviewed worker')
    names = set()
    if len(model['osLibraries']) > 128:
        raise ValueError('model manifest has too many OS dependencies')
    for item in model['osLibraries']:
        if (not isinstance(item, dict) or set(item) != {'name', 'sha256'}
                or not isinstance(item['name'], str)
                or not re.fullmatch(r'[A-Za-z0-9_][A-Za-z0-9_.-]{0,123}\.dll', item['name'], re.IGNORECASE)
                or item['name'].casefold() in names or not digest(item['sha256'])):
            raise ValueError('model manifest has invalid OS dependencies')
        names.add(item['name'].casefold())
    if any(sha(path) != expected for path, expected in
           ((template, template_sha256), (helper, helper_sha256), (worker, worker_sha256))):
        raise ValueError('model manifest input changed during preparation')
    model['helperSha256'] = helper_sha256
    data = (json.dumps(model, sort_keys=True, separators=(',', ':')) + '\n').encode('utf-8')
    output = Path(output)
    with output.open('xb') as destination:
        destination.write(data)
    return sha(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('template', 'helper', 'worker', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('template-sha256', 'helper-sha256', 'worker-sha256'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    result = prepare(args.template, args.template_sha256, args.helper, args.helper_sha256,
                     args.worker, args.worker_sha256, args.output)
    print(json.dumps({'modelManifestSha256': result, 'productionAccepted': False}))


if __name__ == '__main__':
    main()
