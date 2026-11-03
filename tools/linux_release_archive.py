"""Native Linux archive collection. Pins are inputs from the authenticated release job.
No fixture or unsigned payload is admitted; a real external production verifier is required.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile
from linux_sandbox_package import bounded_read, regular_file, stamp, checked, prepare, relative

SHA=re.compile(r"[0-9a-f]{64}")

def unique_object(pairs):
    result={}
    for key,value in pairs:
        if key in result: raise ValueError("duplicate release JSON key")
        result[key]=value
    return result

def json_file(path,limit):return json.loads(bounded_read(path,limit),object_pairs_hook=unique_object)

def immutable_ancestors(path):
    if not path.is_absolute() or path.resolve(strict=True)!=path: raise ValueError("canonical publication path required")
    for entry in (path,*path.parents):
        s=entry.lstat()
        if s.st_uid!=0 or s.st_mode&0o022 or entry.is_symlink(): raise ValueError("root-owned immutable publication ancestors required")

def trusted_verifier(path,digest,bundle):
    path=Path(path)
    if not isinstance(digest,str) or not SHA.fullmatch(digest): raise ValueError("independently authenticated verifier digest required")
    if path.is_relative_to(bundle): raise ValueError("candidate cannot verify itself")
    immutable_ancestors(path)
    with regular_file(path) as image:
        before=os.fstat(image.fileno());h=hashlib.sha256();total=0
        if not 64<=before.st_size<=256<<20: raise ValueError("verifier bounds")
        while chunk:=image.read(65536):
            total+=len(chunk)
            if total>256<<20: raise ValueError("verifier grew")
            h.update(chunk)
        image.seek(0)
        if image.read(4)!=b"\x7fELF" or h.hexdigest()!=digest or stamp(before)!=stamp(os.fstat(image.fileno())) or stamp(before)!=stamp(path.stat()):
            raise ValueError("external verifier image mismatch")
    return path

def immutable_tree(bundle):
    immutable_ancestors(bundle);count=0
    for directory,dirs,files in os.walk(bundle,followlinks=False):
        for name in [directory,*(str(Path(directory)/n)for n in dirs+files)]:
            st=Path(name).lstat();count+=1
            if count>100000 or st.st_uid!=0 or st.st_mode&0o022 or not(stat.S_ISREG(st.st_mode)or stat.S_ISDIR(st.st_mode)):
                raise ValueError("finite root-owned immutable candidate tree required")

def digest_file(path,maximum=512<<20):
    with regular_file(path)as content:
        before=os.fstat(content.fileno());h=hashlib.sha256();total=0
        while chunk:=content.read(65536):
            total+=len(chunk)
            if total>maximum:raise ValueError("archive component size bound")
            h.update(chunk)
        if total!=before.st_size or stamp(before)!=stamp(os.fstat(content.fileno()))or stamp(before)!=stamp(path.stat()):raise ValueError("archive component generation changed")
    return h.hexdigest()

def verify_candidate(bundle,verifier,verifier_sha,committed_roots):
    bundle=Path(bundle);immutable_ancestors(bundle)
    verifier=trusted_verifier(verifier,verifier_sha,bundle)
    ordinary=checked(bundle,"authority/ordinary-authority.json");roots=checked(bundle,"authority/root-keys.json")
    keys=json_file(roots,65536)
    if keys!=json_file(Path(committed_roots),65536): raise ValueError("candidate roots differ from committed release roots")
    if set(keys)!={"version","keys"} or keys["version"]!=1 or not 1<=len(keys["keys"])<=8: raise ValueError("root key bounds")
    args=[str(verifier),"--standalone-authority","--verify-linux-sandbox-release","--standalone-authority-release",str(ordinary)]
    for key in keys["keys"]:
        if not isinstance(key,str) or not re.fullmatch(r"[A-Za-z0-9._-]{1,128}:[0-9a-f]{64}",key): raise ValueError("root key syntax")
        args.extend(["--standalone-authority-key",key])
    with tempfile.TemporaryDirectory(prefix="gent-linux-archive-verifier-") as directory:
        subprocess.run([*args,"--data-dir",directory],env={"PATH":"/usr/bin:/bin","GENT_NODE_BINARY":str(bundle/"runtime/node/bin/node")},stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=45,check=True)

class Components(list):
    def __init__(self,files,expected):super().__init__(files);self.expected=expected

def collect(bundle,descriptor_sha,target,verifier,verifier_sha,committed_roots,base):
    bundle=Path(bundle);immutable_tree(bundle)
    if target not in ("x86_64-unknown-linux-gnu","aarch64-unknown-linux-gnu"): raise ValueError("Linux companion requires native Linux target")
    if not isinstance(descriptor_sha,str) or not SHA.fullmatch(descriptor_sha): raise ValueError("authenticated companion digest required")
    descriptor=checked(bundle,"authority/linux-sandbox.json");raw=bounded_read(descriptor,65536)
    if hashlib.sha256(raw).hexdigest()!=descriptor_sha: raise ValueError("companion pin mismatch")
    envelope=json.loads(raw,object_pairs_hook=unique_object)
    if set(envelope)!={"keyId","payload","signatureHex"} or not re.fullmatch(r"[0-9a-f]{128}",envelope["signatureHex"]): raise ValueError("signed companion required")
    payload=envelope["payload"]
    if payload["platform"]!=target: raise ValueError("companion target mismatch")
    spec={"platform":target,**{key:payload[key]["path"]for key in ("helper","node","gent","network")},"mcpEntries":{key:value["path"]for key,value in payload["mcpEntries"].items()}}
    ordinary=checked(bundle,"authority/ordinary-authority.json")
    if prepare(bundle,spec,hashlib.sha256(bounded_read(ordinary,16<<20)).hexdigest(),payload["expiresAtUnixSeconds"])!=payload: raise ValueError("companion resource generations differ")
    pins=[payload["helper"],payload["node"],payload["gent"],*payload["mcpEntries"].values()]
    # Keep normal binaries/Node/ordinary authority exactly those already selected by packager.
    for name in ("gent","gentd","runtime/node/bin/node","authority/ordinary-authority.json","authority/root-keys.json"):
        if name not in base:raise ValueError("normal release component missing: "+name)
    base_expected={}
    for name in base:
        expected=digest_file(checked(bundle,name))
        if digest_file(Path(base[name]))!=expected: raise ValueError("companion differs from normal package component: "+name)
        base_expected[name]=expected
    names={p["path"]for p in pins}|{"authority/linux-sandbox.json"}
    network=checked(bundle,payload["network"]["path"])
    names|={payload["network"]["path"]+"/"+p.relative_to(network).as_posix()for p in network.rglob("*")if p.is_file()}
    for name in names-{"gent","runtime/node/bin/node","authority/linux-sandbox.json"}:
        if not name.startswith("runtime/linux-sandbox/"): raise ValueError("companion assets must occupy runtime/linux-sandbox only")
    files=[];expected=dict(base_expected)
    for name in sorted(names):
        path=checked(bundle,name)
        digest=digest_file(path)
        if name not in base:files.append((path,name))
        expected[name]=digest
    verify_candidate(bundle,verifier,verifier_sha,committed_roots)
    if bounded_read(descriptor,65536)!=raw:raise ValueError("verified companion changed")
    return Components(files,expected)

def verify_archive(path,root,components):
    import tarfile
    with tarfile.open(path,"r:gz")as package:
        members=package.getmembers();names=[m.name for m in members]
        if len(names)!=len(set(names)):raise ValueError("duplicate archived components")
        for name,digest in components.expected.items():
            entry=package.getmember(root+"/"+name)
            if not entry.isfile()or entry.linkname:raise ValueError("nonregular companion archive member")
            with package.extractfile(entry)as data:
                if hashlib.file_digest(data,"sha256").hexdigest()!=digest:raise ValueError("archived companion generation changed: "+name)

def archive_members(package,root,target):
    """Finite archive consistency, NOT Ed25519 trust or permission evidence."""
    def data(name,limit):
        member=package.getmember(root+"/"+name)
        if not member.isfile()or member.linkname or member.size>limit:raise ValueError("companion archive member bounds")
        with package.extractfile(member)as source:
            value=source.read(limit+1)
            if len(value)>limit:raise ValueError("companion archive member grew")
        return value
    envelope=json.loads(data("authority/linux-sandbox.json",65536),object_pairs_hook=unique_object)
    if set(envelope)!={"keyId","payload","signatureHex"}or not re.fullmatch(r"[0-9a-f]{128}",envelope["signatureHex"]):raise ValueError("signed companion envelope required")
    p=envelope["payload"]
    if p["schema"]!=1 or p["platform"]!=target or p["revoked"]is not False or set(p["mcpEntries"])!={"gent-browser","gent-canvas","mate-canvas"}:raise ValueError("finite companion contract required")
    pins=[p["helper"],p["node"],p["gent"],*p["mcpEntries"].values()]
    names={"authority/linux-sandbox.json"};expected={}
    for pin in pins:
        name=pin["path"];relative(name)
        if name in expected or not isinstance(pin["bytes"],int)or isinstance(pin["bytes"],bool)or not 64<=pin["bytes"]<=512<<20 or not SHA.fullmatch(pin["sha256"]):raise ValueError("finite archive pin required")
        member=package.getmember(root+"/"+name)
        if member.size!=pin["bytes"]or not member.isfile()or member.linkname:raise ValueError("archived pin size/type mismatch")
        with package.extractfile(member)as source:
            if hashlib.file_digest(source,"sha256").hexdigest()!=pin["sha256"]:raise ValueError("archived pin digest mismatch")
        if pin in [p["helper"],p["node"],p["gent"]] and not member.mode&0o111:raise ValueError("archived executable lacks execute permission")
        names.add(name);expected[name]=pin["sha256"]
    if p["node"]["path"]!="runtime/node/bin/node"or p["gent"]["path"]!="gent":raise ValueError("normal archive component mismatch")
    network=p["network"]["path"];relative(network)
    manifest_name=network+"/manifest.json";raw=data(manifest_name,131072)
    if hashlib.sha256(raw).hexdigest()!=p["network"]["manifestSha256"]:raise ValueError("network manifest hash mismatch")
    manifest=json.loads(raw,object_pairs_hook=unique_object)
    if set(manifest)!={"schema","files"}or manifest["schema"]!=1 or not 1<=len(manifest["files"])<=512:raise ValueError("network manifest bounds")
    names.add(manifest_name);network_names=set();total=0
    for entry in manifest["files"]:
        if set(entry)!={"path","sha256"}:raise ValueError("network entry shape")
        relative(entry["path"])
        if entry["path"]in network_names:raise ValueError("network duplicate")
        network_names.add(entry["path"]);name=network+"/"+entry["path"]
        value=data(name,16<<20);total+=len(value)
        if total>64<<20 or hashlib.sha256(value).hexdigest()!=entry["sha256"]:raise ValueError("network file hash/bounds")
        names.add(name)
    required={"usr/bin/slirp4netns","usr/sbin/iptables","usr/sbin/ip6tables","usr/sbin/iptables-restore","usr/sbin/ip6tables-restore"}
    if not required<=network_names:raise ValueError("network executable closure missing")
    for name in required:
        if not package.getmember(root+"/"+network+"/"+name).mode&0o111:raise ValueError("network executable lacks execute permission")
    for name in names-{"gent","runtime/node/bin/node","authority/linux-sandbox.json"}:
        if not name.startswith("runtime/linux-sandbox/"):raise ValueError("companion archive subtree mismatch")
    observed={m.name[len(root)+1:]for m in package.getmembers()if m.name.startswith(root+"/runtime/linux-sandbox/")}
    if observed!={n for n in names if n.startswith("runtime/linux-sandbox/")}:raise ValueError("unmanifested Linux archive files")
    return names
