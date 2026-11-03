#!/usr/bin/env python3
"""Ephemeral CI packaging of an ALREADY genuinely signed bundle. No signer/secrets.
Only a trusted protected-tag caller may supply authenticated artifact digests.
Never install packages, units, trust roots or start services.
"""
import argparse,hashlib,os,re,subprocess,tarfile,tempfile
from pathlib import Path,PurePosixPath
from linux_release_archive import digest_file
from linux_sandbox_package import regular_file,stamp

def copy_verified(source,destination,expected,maximum):
    with regular_file(source)as content,destination.open('xb')as output:
        before=os.fstat(content.fileno());h=hashlib.sha256();total=0
        while chunk:=content.read(65536):
            total+=len(chunk)
            if total>maximum:raise ValueError('input size bound')
            output.write(chunk);h.update(chunk)
        if total!=before.st_size or h.hexdigest()!=expected or stamp(before)!=stamp(os.fstat(content.fileno()))or stamp(before)!=stamp(source.stat()):raise ValueError('authenticated input generation mismatch')
    destination.chmod(0o444)

def unpack(archive,destination):
    total=0;seen=set()
    with tarfile.open(archive,'r:*')as source:
        for member in source:
            parts=PurePosixPath(member.name).parts
            if not parts or member.name.startswith('/')or'\\'in member.name or any(p in ('','.','..')for p in member.name.split('/'))or parts[0]not in ('gent','gentd','runtime','authority')or member.name in seen:raise ValueError('unsafe or duplicate bundle entry')
            seen.add(member.name)
            if len(seen)>100000 or member.size>512<<20 or not(member.isfile()or member.isdir())or member.linkname:raise ValueError('bundle member bounds/type')
            total+=member.size
            if total>2<<30:raise ValueError('bundle aggregate bounds')
            target=destination.joinpath(*parts)
            if member.isdir():target.mkdir(parents=True,exist_ok=True);target.chmod(0o755);continue
            target.parent.mkdir(parents=True,exist_ok=True)
            with source.extractfile(member)as data,target.open('xb')as output:
                remaining=member.size
                while remaining:
                    chunk=data.read(min(65536,remaining))
                    if not chunk:raise ValueError('truncated bundle entry')
                    output.write(chunk);remaining-=len(chunk)
            target.chmod(0o755 if member.mode&0o111 else 0o644)

def publish_outputs(source,destination):
    if not destination.is_absolute():raise ValueError('absolute output directory required')
    destination.mkdir(mode=0o700,exist_ok=True)
    fd=os.open(destination,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
    try:
        for path in source.iterdir():
            if not path.is_file()or path.is_symlink():raise ValueError('regular generated output required')
            output=os.open(path.name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o644,dir_fd=fd)
            with os.fdopen(output,'wb')as target,regular_file(path)as content:
                before=os.fstat(content.fileno());total=0
                while chunk:=content.read(65536):
                    total+=len(chunk)
                    if total>2<<30:raise ValueError('generated artifact bounds')
                    target.write(chunk)
                if total!=before.st_size or stamp(before)!=stamp(os.fstat(content.fileno())):raise ValueError('generated artifact changed')
                target.flush();os.fsync(target.fileno())
    finally:os.close(fd)

def main():
    p=argparse.ArgumentParser()
    for name in ('bundle','verifier','out-dir'):p.add_argument('--'+name,type=Path,required=True)
    for name in ('bundle-sha256','verifier-sha256','descriptor-sha256','version','target'):p.add_argument('--'+name,required=True)
    a=p.parse_args()
    if os.geteuid()!=0:raise ValueError('ephemeral CI root-owned staging required; this command never installs')
    if a.target not in ('x86_64-unknown-linux-gnu','aarch64-unknown-linux-gnu')or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+',a.version):raise ValueError('finite release target/version required')
    for digest in (a.bundle_sha256,a.verifier_sha256,a.descriptor_sha256):
        if not re.fullmatch('[0-9a-f]{64}',digest):raise ValueError('authenticated upstream digests required')
    tools=Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix='gent-linux-ci-',dir='/root')as directory:
        root=Path(directory);bundle=root/'bundle';bundle.mkdir(mode=0o755)
        archive=root/'input.tar';copy_verified(a.bundle,archive,a.bundle_sha256,2<<30);unpack(archive,bundle)
        verifier=root/'verifier';copy_verified(a.verifier,verifier,a.verifier_sha256,256<<20);verifier.chmod(0o555)
        output=root/'output';output.mkdir(mode=0o700)
        env={'PATH':'/usr/bin:/bin','SOURCE_DATE_EPOCH':os.environ.get('SOURCE_DATE_EPOCH','0')}
        subprocess.run(['/usr/bin/python3',str(tools/'package-release.py'),'--target-dir',str(bundle),'--out-dir',str(output),'--version',a.version,'--target',a.target,'--format','tar.gz','--node-runtime-dir',str(bundle/'runtime/node'),'--claurst-runtime-dir',str(bundle/'runtime/claurst'),'--authority-release',str(bundle/'authority/ordinary-authority.json'),'--authority-root-keys',str(bundle/'authority/root-keys.json'),'--linux-sandbox-bundle-root',str(bundle),'--linux-sandbox-descriptor-sha256',a.descriptor_sha256,'--linux-sandbox-verifier',str(verifier),'--linux-sandbox-verifier-sha256',a.verifier_sha256],env=env,check=True,timeout=180)
        subprocess.run(['/usr/bin/python3',str(tools/'package-linux-service-deb.py'),'--bundle',str(bundle),'--verifier',str(verifier),'--verifier-sha256',a.verifier_sha256,'--version',a.version,'--target',a.target,'--output',str(output/('gent-contained-'+a.version+'-'+a.target+'.deb'))],env=env,check=True,timeout=180)
        publish_outputs(output,a.out_dir)

if __name__=='__main__':main()
