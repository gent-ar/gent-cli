"""Measure finite Claude Windows resources and optional policy candidate; never admits/executes a provider."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re


def sha(path):
    with path.open('rb') as source:return hashlib.file_digest(source,'sha256').hexdigest()


def relative(value):
    if not value or len(value)>256 or any(not re.fullmatch('[A-Za-z0-9_.@-]+',p) or p in ('.','..') or p.endswith('.') for p in value.split('/')):
        raise ValueError('invalid finite resource path')
    return value


def physical(root,name):
    root=Path(root)
    if root.is_symlink() or getattr(root.stat(),'st_file_attributes',0)&0x400:raise ValueError('redirected resource root')
    path=root/relative(name)
    for candidate in [path,*list(path.parents)[:len(path.relative_to(root).parts)-1]]:
        if candidate.is_symlink() or getattr(candidate.stat(),'st_file_attributes',0)&0x400:raise ValueError('redirected finite resource')
    if not path.is_file():raise ValueError('finite resource is not an ordinary file')
    return path


def generate(package_root,node_root,git_root,executable,integrity,package_files,libraries):
    if not re.fullmatch(r'sha512-[A-Za-z0-9+/]+={0,2}',integrity):raise ValueError('explicit approved package integrity is required')
    if len(base64.b64decode(integrity[7:],validate=True))!=64:raise ValueError('invalid SHA512 package integrity')
    metadata_path=physical(package_root,'package.json')
    if metadata_path.stat().st_size>1024*1024:raise ValueError('package metadata exceeds candidate bound')
    metadata=json.loads(metadata_path.read_text(encoding='utf-8'))
    if metadata.get('name')!='@anthropic-ai/claude-code-win32-x64' or not isinstance(metadata.get('version'),str) or len(metadata['version'])>128 or not re.fullmatch(r'\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?',metadata['version']):raise ValueError('approved native Claude Windows package identity is required')
    if executable!='claude.exe':raise ValueError('Claude native executable must match managed platform layout')
    if len(package_files)>249:raise ValueError('finite resource count exceeds native contract')
    names=['package.json',executable]+list(package_files)
    if len({x.lower() for x in names})!=len(names):raise ValueError('duplicate finite package paths')
    files=[];measured=[]
    for kind,root,paths in [('package',package_root,names),('nodeRuntime',node_root,['bin/node.exe','bin/npm.cmd','lib/node_modules/npm/bin/npm-cli.js']),('gitRuntime',git_root,['usr/bin/bash.exe','usr/bin/msys-2.0.dll','etc/fstab'])]:
        for name in paths:
            path=physical(root,name);digest=sha(path)
            if kind=='gitRuntime' and name=='etc/fstab' and digest!='af2184544d99dd656f4e5e119d80f2cb15f03c29d3d8467da1df5aa330508f4a':raise ValueError('Git temporary mount configuration differs from reviewed mapping')
            files.append(dict(root=kind,path=name,sha256=digest));measured.append(dict(path=str(path.resolve()),sha256=digest))
    libraries=[n.lower() for n in libraries]
    if not libraries or len(libraries)>128 or len(set(libraries))!=len(libraries) or any(not re.fullmatch('[a-z0-9_-][a-z0-9_.-]*\\.dll',n) or '..' in n or n.startswith(('api-ms-','ext-ms-')) for n in libraries):raise ValueError('explicit unique physical OS DLL closure is required')
    if len(files)>256:raise ValueError('finite resource count exceeds native contract')
    entry=dict(provider='claude',platform='win32-x64',packageName=metadata['name'],packageVersion=metadata['version'],packageIntegrity=integrity,nodeSha256=sha(physical(node_root,'bin/node.exe')),executableSha256=sha(physical(package_root,executable)),credentialContract='claude-loopback-v1',files=files,osLibraries=sorted(libraries))
    for row in measured:
        if sha(Path(row['path']))!=row['sha256']:raise ValueError('resource changed during candidate measurement')
    return dict(version=1,providers=[entry]),measured


def write_candidate(output,manifest,measured,helper=None,authority=None,authority_pin=None):
    if any([helper,authority,authority_pin]) and not all([helper,authority,authority_pin]):raise ValueError('descriptor candidate requires helper and reviewed package authority with SHA256')
    descriptor=None
    if authority:
        if authority.stat().st_size>16384 or not re.fullmatch('[0-9a-f]{64}',authority_pin) or sha(authority)!=authority_pin:raise ValueError('package authority differs from reviewed input pin')
        report=json.loads(authority.read_text(encoding='utf-8'));package=report.get('package',{})
        if report.get('schema')!=1 or package.get('developmentRegistration') is not False:raise ValueError('signed production package authority candidate is required')
        if set(package)!={'family','fullName','application','imageSha256','manifestSha256','developmentRegistration'} or any(not isinstance(package.get(k),str) or not re.fullmatch('[A-Za-z0-9_.-]{1,240}',package[k]) for k in ['family','fullName','application']) or any(not isinstance(package.get(k),str) or not re.fullmatch('[0-9a-f]{64}',package[k]) for k in ['imageSha256','manifestSha256']):raise ValueError('invalid reviewed package identity candidate')
        descriptor=dict(schema=1,policy='external-identity-proxy-private-v1',provider='claude',ingress='private-network-capability-accepted',physicalLanVerified=False,helperSha256=sha(helper),resourcesSha256='',package=package)
    if output.exists():raise FileExistsError('candidate output must be fresh')
    output.mkdir(parents=True);resources=output/'candidate-provider-resources-claude.json';resources.write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n',encoding='utf-8')
    if descriptor:
        descriptor['resourcesSha256']=sha(resources);(output/'candidate-external-proxy-policy-claude.json').write_text(json.dumps(descriptor,indent=2,sort_keys=True)+'\n',encoding='utf-8')
    receipt=dict(schema=1,provider='claude',admitted=False,policyEnablementPerformed=False,resourcesSha256=sha(resources),measuredInputs=measured,scope='Measurement candidate only. Complete loader/dynamic/tool closure, credential gateway and real contained factory acceptance are still required. No execution, auth probe or policy enablement.')
    (output/'candidate-measurement.json').write_text(json.dumps(receipt,indent=2,sort_keys=True)+'\n',encoding='utf-8')
    return receipt


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['package-root','node-runtime','git-runtime','out']:p.add_argument('--'+n,type=Path,required=True)
    p.add_argument('--executable',default='claude.exe');p.add_argument('--package-integrity',required=True);p.add_argument('--package-file',action='append',default=[]);p.add_argument('--os-library',action='append',required=True);p.add_argument('--helper',type=Path);p.add_argument('--package-authority',type=Path);p.add_argument('--package-authority-sha256');a=p.parse_args();manifest,measured=generate(a.package_root,a.node_runtime,a.git_runtime,a.executable,a.package_integrity,a.package_file,a.os_library);receipt=write_candidate(a.out,manifest,measured,a.helper,a.package_authority,a.package_authority_sha256);print(json.dumps(dict(out=str(a.out),resourcesSha256=receipt['resourcesSha256'],admitted=False)))
if __name__=='__main__':main()
