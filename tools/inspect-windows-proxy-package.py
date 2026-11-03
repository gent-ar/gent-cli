"""Verify a signed proxy artifact and emit native helper build inputs. Never installs/signs/activates."""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import zipfile
from xml.etree import ElementTree as ET
from windows_sandbox_package import ordinary, sha, trusted_package

F='{http://schemas.microsoft.com/appx/manifest/foundation/windows10}'
U='{http://schemas.microsoft.com/appx/manifest/uap/windows10/10}'
D='{http://schemas.microsoft.com/appx/manifest/desktop/windows10/2}'
IMAGE='data/gent-network-proxy/gent-network-proxy.exe'


def native_identity(identity):
    if os.name!='nt':
        raise ValueError('canonical package identity requires Windows appmodel APIs')
    class PackageId(ctypes.Structure):
        _fields_=[('reserved',wintypes.UINT),('architecture',wintypes.UINT),('version',ctypes.c_uint64),('name',wintypes.LPWSTR),('publisher',wintypes.LPWSTR),('resource',wintypes.LPWSTR),('publisher_id',wintypes.LPWSTR)]
    parts=identity['Version'].split('.')
    if len(parts)!=4 or any(not x.isdigit() or int(x)>65535 for x in parts):
        raise ValueError('invalid package version')
    if identity.get('ProcessorArchitecture')!='x64':
        raise ValueError('proxy package must be x64')
    version=sum(int(value)<<(48-16*i) for i,value in enumerate(parts))
    value=PackageId(0,9,version,identity['Name'],identity['Publisher'],identity.get('ResourceId',''),None)
    kernel=ctypes.WinDLL('kernel32',use_last_error=True)
    def query(name):
        api=getattr(kernel,name);api.argtypes=[ctypes.POINTER(PackageId),ctypes.POINTER(wintypes.UINT),wintypes.LPWSTR];api.restype=wintypes.LONG
        count=wintypes.UINT();status=api(ctypes.byref(value),ctypes.byref(count),None)
        if status!=122 or not 1<count.value<4096:raise ValueError(name+' identity size query failed: '+str(status))
        buffer=ctypes.create_unicode_buffer(count.value);status=api(ctypes.byref(value),ctypes.byref(count),buffer)
        if status:raise ValueError(name+' failed: '+str(status))
        return buffer.value
    family=query('PackageFamilyNameFromId');value.publisher_id=family.rsplit('_',1)[1]
    return family,query('PackageFullNameFromId')


def inspect(package, signtool, verifier=trusted_package, identity_query=native_identity):
    package=ordinary(Path(package));before=sha(package);verifier(package,signtool)
    with zipfile.ZipFile(package) as archive:
        names=archive.namelist()
        if len(names)!=len(set(names)) or 'AppxSignature.p7x' not in names:raise ValueError('proxy package signature/layout missing or ambiguous')
        info=archive.getinfo('AppxManifest.xml')
        if info.file_size>1024*1024:raise ValueError('proxy manifest exceeds bound')
        manifest=archive.read(info);root=ET.fromstring(manifest);identity=root.find(F+'Identity')
        if identity is None:raise ValueError('proxy package has no Identity')
        app=next((a for a in root.findall(F+'Applications/'+F+'Application') if a.get('Id')=='GentNetworkProxy'),None)
        if app is None or app.get('Executable','').replace('\\','/')!=IMAGE or any(app.get(U+key)!=value for key,value in [('TrustLevel','appContainer'),('RuntimeBehavior','packagedClassicApp'),('SupportsMultipleInstances','true')]):raise ValueError('proxy Application does not match reviewed AppContainer/multiinstance contract')
        capabilities={x.get('Name') for x in root.findall(F+'Capabilities/*')}
        if capabilities!={'privateNetworkClientServer','runFullTrust'}:raise ValueError('proxy capability set differs from private bridge contract')
        blocks=[x for x in root.findall('.//'+D+'FirewallRules') if x.get('Executable','').replace('\\','/')==IMAGE]
        if len(blocks)!=1 or len(list(blocks[0]))!=1:raise ValueError('proxy firewall declaration is ambiguous')
        rule=list(blocks[0])[0]
        if rule.attrib!={'Direction':'in','IPProtocol':'TCP','Profile':'all','LocalPortMin':'55490','LocalPortMax':'55521'}:raise ValueError('proxy firewall pool differs from native allocator')
        image=archive.getinfo(IMAGE)
        if image.file_size>16*1024*1024:raise ValueError('proxy image exceeds bound')
        payload=archive.read(image)
        if len(payload)<256 or payload[:2]!=b'MZ':raise ValueError('proxy image is not an x64 executable')
        pe=struct.unpack_from('<I',payload,60)[0]
        if pe>len(payload)-112 or payload[pe:pe+4]!=b'PE\0\0' or struct.unpack_from('<H',payload,pe+4)[0]!=0x8664 or struct.unpack_from('<H',payload,pe+22)[0]&0x2000 or struct.unpack_from('<H',payload,pe+24)[0]!=0x20b:raise ValueError('proxy image is not an x64 executable')
        image_hash=hashlib.sha256(payload).hexdigest()
        family,full=identity_query(identity.attrib)
    if sha(package)!=before:raise ValueError('proxy artifact changed during trust/identity inspection')
    for value in [family,full,'GentNetworkProxy']:
        if not re.fullmatch('[A-Za-z0-9_.-]{1,240}',value):raise ValueError('unsupported canonical package identifier')
    return dict(schema=1,packageSha256=before,signatureVerification='Windows SDK SignTool verify /pa /all exit0',trustScope='Current Windows trust store; clean-target trust acceptance remains required',identity=dict(identity.attrib),installed=False,published=False,
        package=dict(family=family,fullName=full,application='GentNetworkProxy',imageSha256=image_hash,manifestSha256=hashlib.sha256(manifest).hexdigest(),developmentRegistration=False))


def write_inputs(report,output,helper=None,resources=None):
    if bool(helper)!=bool(resources):raise ValueError('helper and resource manifest must be supplied together')
    if output.exists():raise FileExistsError('refuse replacing existing authority output')
    output.mkdir(parents=True);(output/'package-authority.json').write_text(json.dumps(report,indent=2,sort_keys=True)+'\n',encoding='utf-8')
    package=report['package'];fields={'FAMILY':package['family'],'FULL_NAME':package['fullName'],'APPLICATION':package['application'],'IMAGE_SHA256':package['imageSha256'],'MANIFEST_SHA256':package['manifestSha256'],'IMAGE':IMAGE}
    lines=['# Generated from a verified signed artifact. This does not install or enable it.']
    lines += ['set(GENT_EXTERNAL_PACKAGE_'+key+' "'+value+'" CACHE STRING "Verified signed package authority" FORCE)' for key,value in fields.items()]
    lines += ['set(GENT_EXTERNAL_PACKAGE_ALLOW_DEVELOPMENT_FOR_TEST OFF CACHE BOOL "Production package only" FORCE)']
    (output/'package-authority.cmake').write_text('\n'.join(lines)+'\n',encoding='utf-8')
    if helper is not None:
        helper_hash=sha(ordinary(Path(helper)));resource_hash=sha(ordinary(Path(resources)))
        descriptor=dict(schema=1,policy='external-identity-proxy-private-v1',provider='codex',
                        ingress='private-network-capability-accepted',physicalLanVerified=False,
                        helperSha256=helper_hash,resourcesSha256=resource_hash,package=package)
        (output/'reviewed-external-proxy-policy.json').write_text(json.dumps(descriptor,indent=2,sort_keys=True)+'\n',encoding='utf-8')



def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--package',type=Path,required=True);p.add_argument('--sign-tool',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--helper',type=Path);p.add_argument('--resources',type=Path);a=p.parse_args();report=inspect(a.package,a.sign_tool);write_inputs(report,a.out,a.helper,a.resources);print(json.dumps(dict(out=str(a.out),packageSha256=report['packageSha256'],descriptorSha256=sha(a.out/'reviewed-external-proxy-policy.json') if a.helper else None,installed=False,published=False)))
if __name__=='__main__':main()
