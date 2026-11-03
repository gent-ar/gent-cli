#!/usr/bin/env python3
"""Build a root-owned dpkg artifact only; never install, enable, sign, or start it."""
import argparse
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from linux_release_archive import verify_candidate,immutable_tree,digest_file,json_file
from linux_service_unit import render_installed

def main():
    p=argparse.ArgumentParser()
    p.add_argument("--bundle",type=Path,required=True)
    p.add_argument("--verifier",type=Path,required=True)
    p.add_argument("--verifier-sha256",required=True)
    p.add_argument("--version",required=True)
    p.add_argument("--target",choices=["x86_64-unknown-linux-gnu","aarch64-unknown-linux-gnu"],required=True)
    p.add_argument("--output",type=Path,required=True)
    args=p.parse_args()
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+",args.version)or args.output.exists():raise ValueError("new output and finite version required")
    immutable_tree(args.bundle)
    if json_file(args.bundle/"authority/linux-sandbox.json",65536)["payload"]["platform"]!=args.target:
        raise ValueError("Debian target differs from signed companion")
    roots=Path(__file__).resolve().parents[1]/"platform/authority/root-keys.json"
    verify_candidate(args.bundle,args.verifier,args.verifier_sha256,roots)
    _build_verified_payload(args,_dependency_field(args.bundle))

def _dependency_field(bundle):
    """Use distro-maintained ELF dependency metadata; never execute candidate binaries."""
    images=[];directories=set()
    for path in bundle.rglob('*'):
        if not path.is_file():continue
        with path.open('rb')as content:elf=content.read(4)==b'\x7fELF'
        if elf:images.append(path);directories.add(path.parent)
    if not images or len(images)>2048:raise ValueError('finite native ELF closure required for Debian dependency resolution')
    with tempfile.TemporaryDirectory(prefix='gent-debian-dependencies-')as directory:
        cwd=Path(directory);(cwd/'debian').mkdir()
        (cwd/'debian/control').write_text('Source: gent-runtime-contained\nSection: utils\nPriority: optional\nMaintainer: Gent release maintainers\n\nPackage: gent-runtime-contained\nArchitecture: any\nDescription: Gent runtime\n')
        with tempfile.TemporaryFile()as output:
            result=subprocess.run(['/usr/bin/dpkg-shlibdeps','-O','--package=gent-runtime-contained','-S'+str(bundle),*['-l'+str(p)for p in sorted(directories)],*['-e'+str(p)for p in images]],cwd=cwd,env={'PATH':'/usr/bin:/bin'},stdin=subprocess.DEVNULL,stdout=output,stderr=subprocess.DEVNULL,timeout=45)
            output.seek(0);raw=output.read(65537)
        if result.returncode or len(raw)>65536:raise ValueError('distro dependency resolution failed; use matching build dependencies and complete private library closure')
    lines=raw.decode('ascii').splitlines()
    fields=[line.removeprefix('shlibs:Depends=')for line in lines if line.startswith('shlibs:Depends=')]
    if len(fields)!=1 or any(ord(c)<32 for c in fields[0]):raise ValueError('unexpected distro dependency output')
    return 'systemd (>= 254)'+(', '+fields[0]if fields[0]else'')

def _build_verified_payload(args,depends):
    """Packaging primitive only: main MUST verify genuine authority before this call."""
    expected={p.relative_to(args.bundle).as_posix():digest_file(p)for p in args.bundle.rglob('*')if p.is_file()}
    with tempfile.TemporaryDirectory(prefix='gent-root-owned-package-',dir=args.output.parent)as directory:
        temporary=Path(directory);stage=temporary/'root';stage.mkdir();release=stage/'usr/lib/gent'/args.version
        shutil.copytree(args.bundle,release,symlinks=False)
        for name,digest in expected.items():
            if digest_file(release/name)!=digest or digest_file(args.bundle/name)!=digest:raise ValueError("package generation changed")
        units=stage/'usr/lib/systemd/user';units.mkdir(parents=True)
        (units/'gent-contained.service').write_text(render_installed(args.version,254))
        control=stage/'DEBIAN';control.mkdir()
        arch='amd64'if args.target.startswith('x86_64')else'arm64'
        (control/'control').write_text('Package: gent-runtime-contained\nVersion: '+args.version+'\nArchitecture: '+arch+'\nMaintainer: Gent release maintainers\nDepends: '+depends+'\nDescription: Gent runtime with explicit delegated user-service startup\n')
        # No maintainer scripts: no daemon-reload, auto-enable, service startup or trust mutation.
        subprocess.run(['/usr/bin/dpkg-deb','--root-owner-group','--build',str(stage),str(temporary/'result.deb')],env={'PATH':'/usr/bin:/bin','SOURCE_DATE_EPOCH':os.environ.get('SOURCE_DATE_EPOCH','0')},stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,timeout=90,check=True)
        if {p.relative_to(args.bundle).as_posix():digest_file(p)for p in args.bundle.rglob('*')if p.is_file()}!=expected:raise ValueError("candidate changed during package creation")
        # Atomic no-replace publication on the same filesystem, after all checks.
        os.link(temporary/'result.deb',args.output)

if __name__=='__main__':main()
