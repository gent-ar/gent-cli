import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess

from windows_sandbox_package import collect, ordinary, sha, read_provider_manifest


TARGET = 'x86_64-pc-windows-msvc'
PINS = {
    'GENT_WINDOWS_SANDBOX_HELPER_SHA256': 'provider-helper.exe',
    'GENT_WINDOWS_PROVIDER_RESOURCES_SHA256': 'provider-resources.json',
    'GENT_WINDOWS_EXTERNAL_PROXY_POLICY_SHA256': 'reviewed-external-proxy-policy.json',
    'GENT_WINDOWS_GIT_BASH_SHA256': 'git/usr/bin/bash.exe',
}


def prepare(runtime, descriptor_pin, package, signtool, output, collector=collect):
    output = Path(output).absolute()
    if output.exists():
        raise ValueError('sandbox release stage must be new; existing output is never replaced')
    components = collector(runtime, descriptor_pin, package, signtool, TARGET)
    output.mkdir(parents=True)
    for source, relative in components:
        destination = output / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        if sha(destination) != components.expected[relative]:
            raise ValueError('sandbox component changed during staging: ' + relative)
    staged = output / 'runtime/windows'
    collector(staged, descriptor_pin, staged / 'proxy-package.msix', signtool, TARGET)
    pins = {name: sha(ordinary(staged / relative)) for name, relative in PINS.items()}
    contracts = read_provider_manifest(staged / 'provider-resources.json')['providers']
    if any(p['provider'] == 'claurst' for p in contracts):
        model = output / 'runtime/claurst/model-server/model-runtime.json'
        pins['GENT_CONTAINED_MODEL_MANIFEST_SHA256'] = sha(ordinary(model))
    if pins['GENT_WINDOWS_EXTERNAL_PROXY_POLICY_SHA256'] != descriptor_pin:
        raise ValueError('staged descriptor differs from reviewed pin')
    proof = {
        'schemaVersion': 1,
        'phase': 'prepared',
        'target': TARGET,
        'cargoEnvironment': pins,
        'sandboxFiles': components.expected,
        'hostOsDataFiles': [f for p in read_provider_manifest(staged / 'provider-resources.json')['providers']
                            for f in p['files'] if f.get('root') == 'osGlobalizationData'],
        'installedAcceptance': False,
        'published': False,
    }
    write_proof(output, proof)
    return output, proof


def write_proof(output, proof):
    (output / 'build-binding.json').write_text(
        json.dumps(proof, indent=2, sort_keys=True) + '\n', encoding='utf-8')


def build(source, target_dir, cargo, output, proof, runner=subprocess.run):
    source = Path(source).resolve(strict=True)
    target_dir = Path(target_dir).absolute()
    manifest = ordinary(source / 'Cargo.toml')
    environment = os.environ.copy()
    environment.pop('GENT_CONTAINED_MODEL_MANIFEST_SHA256', None)
    environment.update(proof['cargoEnvironment'])
    command = [str(cargo), 'build', '--manifest-path', str(manifest), '--locked',
               '--release', '--target', TARGET, '--target-dir', str(target_dir), '-j1',
               '-p', 'gent-cli', '-p', 'gentd', '--bins']
    proof['command'] = command
    proof['phase'] = 'building'
    write_proof(output, proof)
    runner(command, cwd=source, env=environment, stdin=subprocess.DEVNULL, check=True)
    for relative, expected in proof['sandboxFiles'].items():
        if sha(ordinary(output / relative)) != expected:
            raise ValueError('sandbox component changed during build: ' + relative)
    binaries = {}
    for name in ('gent.exe', 'gentd.exe', 'gent-launcher.exe'):
        original = ordinary(target_dir / TARGET / 'release' / name)
        expected = sha(original)
        destination = output / 'bin' / name
        destination.parent.mkdir(exist_ok=True)
        shutil.copyfile(original, destination)
        if sha(destination) != expected:
            raise ValueError('release binary changed during staging: ' + name)
        binaries[name] = expected
    proof.update(phase='built', binaries=binaries)
    write_proof(output, proof)


def arguments():
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument('--target-dir', type=Path, required=True)
    parser.add_argument('--cargo', type=Path, required=True)
    parser.add_argument('--runtime', type=Path, required=True)
    parser.add_argument('--descriptor-sha256', required=True)
    parser.add_argument('--proxy-package', type=Path, required=True)
    parser.add_argument('--sign-tool', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--prepare-only', action='store_true')
    return parser.parse_args()


def main():
    if os.name != 'nt':
        raise SystemExit('Windows sandbox release build requires a Windows host')
    args = arguments()
    output, proof = prepare(args.runtime, args.descriptor_sha256, args.proxy_package,
                            args.sign_tool, args.out)
    if not args.prepare_only:
        build(args.source, args.target_dir, args.cargo, output, proof)
    print(json.dumps({'stage': str(output), 'phase': proof['phase'],
                      'installedAcceptance': False, 'published': False}))


if __name__ == '__main__':
    main()
