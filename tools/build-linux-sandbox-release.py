#!/usr/bin/env python3
"""Prepare or sign a companion with the existing ordinary release root signer.
Does not install resources, mutate trust, or attest OS capability.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import stat
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from linux_sandbox_package import prepare, compact, bounded_read, stamp, regular_file


def main():
    p=argparse.ArgumentParser()
    p.add_argument("--bundle",type=Path,required=True)
    p.add_argument("--resources",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    p.add_argument("--expires",type=int,required=True)
    p.add_argument("--sign",action="store_true")
    p.add_argument("--trusted-verifier",type=Path)
    p.add_argument("--trusted-verifier-sha256")
    a=p.parse_args()
    if sys.platform!="linux": raise ValueError("Linux companion preparation/signing requires native Linux")
    if a.output.exists(): raise ValueError("output already exists; never replace a candidate")
    if not int(time.time()) < a.expires <= int(time.time())+366*86400: raise ValueError("expiry outside bounded release interval")
    ordinary=a.bundle/"authority/ordinary-authority.json"
    if ordinary.resolve(strict=True)!=ordinary or ordinary.is_symlink(): raise ValueError("ordinary authority alias")
    raw=bounded_read(ordinary,16<<20)
    if len(raw)>16<<20: raise ValueError("ordinary authority bounds")
    payload=prepare(a.bundle,json.loads(bounded_read(a.resources,65536)),hashlib.sha256(raw).hexdigest(),a.expires)
    result={"preparedPayload":payload,"signed":False,"capabilityAvailable":False}
    if a.sign:
        if os.name!="posix": raise ValueError("sign candidate verification requires native Linux")
        # Never execute a candidate as its own verifier on the signing host.
        if a.trusted_verifier is None or a.trusted_verifier_sha256 is None:
            raise ValueError("signing requires an independently authenticated external verifier")
        verifier=a.trusted_verifier
        if not verifier.is_absolute() or verifier.resolve(strict=True)!=verifier or verifier.is_relative_to(a.bundle):
            raise ValueError("trusted verifier must be canonical and external to candidate")
        for path in (verifier,*verifier.parents):
            st=path.lstat()
            if st.st_uid!=0 or st.st_mode & 0o022 or path.is_symlink():
                raise ValueError("verifier and ancestors must be root-owned and immutable")
        with regular_file(verifier) as image:
            before=os.fstat(image.fileno())
            if not 64<=before.st_size<=256<<20: raise ValueError("verifier size bounds")
            hasher=hashlib.sha256();total=0
            while chunk:=image.read(65536):
                total+=len(chunk)
                if total>256<<20: raise ValueError("verifier grew beyond bound")
                hasher.update(chunk)
            digest=hasher.hexdigest()
            image.seek(0)
            if image.read(4)!=b"\x7fELF" or digest!=a.trusted_verifier_sha256 or stamp(before)!=stamp(os.fstat(image.fileno())) or stamp(before)!=stamp(verifier.stat()):
                raise ValueError("external verifier identity rejected")
        # Root-owned immutable origin prevents replacement after closing descriptor.
        # The expected digest must come from independent authenticated release provenance.
        env={k:os.environ[k] for k in ("PATH","LANG","LC_ALL") if k in os.environ}
        if a.output != a.bundle/"authority/linux-sandbox.json":
            raise ValueError("signed output must use the exact companion package layout")
        # Publication candidate is frozen against unprivileged changes BEFORE any verification.
        # Root/admin mutations remain outside this trust boundary; generation rechecks follow.
        for path in (a.bundle,*a.bundle.parents):
            st=path.lstat()
            if st.st_uid!=0 or st.st_mode & 0o022 or path.is_symlink():
                raise ValueError("publication root and ALL ancestors must be root-owned and immutable")
        for file in (payload["helper"],payload["node"],payload["gent"]):
            path=a.bundle/file["path"]
            if not path.stat().st_mode & 0o111:
                raise ValueError("packaged executable lacks execution permission: "+file["path"])
            with path.open("rb") as image:
                if image.read(4)!=b"\x7fELF": raise ValueError("packaged executable is not native ELF")
        for name in ("usr/bin/slirp4netns","usr/sbin/iptables","usr/sbin/ip6tables","usr/sbin/iptables-restore","usr/sbin/ip6tables-restore"):
            if not (a.bundle/payload["network"]["path"]/name).stat().st_mode & 0o111:
                raise ValueError("network executable lacks execution permission: "+name)
        count=0
        for directory, dirs, files in os.walk(a.bundle,followlinks=False):
            for name in [directory,*(str(Path(directory)/n) for n in dirs+files)]:
                path=Path(name);st=path.lstat();count+=1
                if count>100000 or st.st_uid!=0 or st.st_mode & 0o022 or path.is_symlink() or not (stat.S_ISREG(st.st_mode) or stat.S_ISDIR(st.st_mode)):
                    raise ValueError("publication tree must be finite root-owned and immutable")
        env["GENT_NODE_BINARY"]=str(a.bundle/"runtime/node/bin/node")
        roots=a.bundle/"authority/root-keys.json"
        committed=Path(__file__).resolve().parents[1]/"platform/authority/root-keys.json"
        if json.loads(bounded_read(roots,65536)) != json.loads(bounded_read(committed,65536)): raise ValueError("candidate roots differ from committed ordinary trust roots")
        args=[str(verifier),"--standalone-authority","--verify-standalone-authority-release","--standalone-authority-release",str(ordinary)]
        for key in json.loads(bounded_read(roots,65536))["keys"]: args.extend(["--standalone-authority-key",key])
        with tempfile.TemporaryDirectory(prefix="gent-companion-verifier-") as data:
            subprocess.run([*args,"--data-dir",data],env=env,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=45,check=True)
        spec=importlib.util.spec_from_file_location("ordinary_signer",Path(__file__).with_name("build-ordinary-authority.py"))
        signer=importlib.util.module_from_spec(spec); spec.loader.exec_module(signer)
        if bounded_read(ordinary,16<<20)!=raw or prepare(a.bundle,json.loads(bounded_read(a.resources,65536)),hashlib.sha256(raw).hexdigest(),a.expires)!=payload:
            raise ValueError("verified candidate generation changed")
        key_id,seed=signer.root_signing_key(committed)
        # Rust canonical Payload serialization preserves this exact field order.
        result={"keyId":key_id,"payload":payload,"signatureHex":signer.SIGNER.sign_seed(seed,compact(payload))}
    output_fd=os.open(a.output,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o600)
    with os.fdopen(output_fd,"wb") as output:
        written=compact(result)+b"\n"
        output.write(written);output.flush();os.fsync(output.fileno())
        output_identity=stamp(os.fstat(output.fileno()))
    if a.sign:
        # The external verifier must include the new read-only companion CLI.
        # Failure never yields a usable published signature from this producer.
        try:
            with tempfile.TemporaryDirectory(prefix="gent-companion-postverify-") as data:
                post=[x for x in args if x!="--verify-standalone-authority-release"]
                subprocess.run([*post,"--verify-linux-sandbox-release","--data-dir",data],env=env,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=45,check=True)
            if bounded_read(ordinary,16<<20)!=raw or prepare(a.bundle,json.loads(bounded_read(a.resources,65536)),hashlib.sha256(raw).hexdigest(),a.expires)!=payload or bounded_read(a.output,65536)!=written:
                raise ValueError("post-verification publication generation changed")
        except BaseException:
            # Delete only the exact output generation that THIS invocation created.
            if a.output.exists() and stamp(a.output.stat())==output_identity:
                a.output.unlink()
            raise

if __name__=="__main__":
    main()

