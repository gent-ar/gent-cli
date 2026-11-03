"""Archive consistency fixtures ONLY; zero signatures/authority are authenticated."""
import io,json,hashlib,tarfile,tempfile,unittest,importlib.util,os
from pathlib import Path
from linux_release_archive import archive_members,trusted_verifier,verify_archive,Components
from linux_service_unit import render_installed

class ArchiveTests(unittest.TestCase):
 def fixture(self,mutation=None):
  data=b'x'*128;digest=hashlib.sha256(data).hexdigest();paths=['runtime/linux-sandbox/helper','runtime/node/bin/node','gent',*[f'runtime/linux-sandbox/mcp/{r}.js'for r in ('gent-browser','gent-canvas','mate-canvas')]]
  files={p:(data,0o755)for p in paths};pin=lambda p:{'path':p,'sha256':digest,'bytes':128}
  required=['usr/bin/slirp4netns','usr/sbin/iptables','usr/sbin/ip6tables','usr/sbin/iptables-restore','usr/sbin/ip6tables-restore']
  network='runtime/linux-sandbox/network';manifest=json.dumps({'schema':1,'files':[{'path':p,'sha256':digest}for p in required]}).encode()
  files.update({network+'/'+p:(data,0o755)for p in required});files[network+'/manifest.json']=(manifest,0o644)
  payload={'schema':1,'platform':'x86_64-unknown-linux-gnu','revoked':False,'helper':pin(paths[0]),'node':pin(paths[1]),'gent':pin(paths[2]),'network':{'path':network,'manifestSha256':hashlib.sha256(manifest).hexdigest()},'mcpEntries':{r:pin(paths[i+3])for i,r in enumerate(('gent-browser','gent-canvas','mate-canvas'))}}
  # Deliberately invalid signature: this is NOT collector/production authorization.
  files['authority/linux-sandbox.json']=(json.dumps({'keyId':'consistency-only','payload':payload,'signatureHex':'00'*64}).encode(),0o644)
  if mutation:mutation(files)
  buffer=io.BytesIO()
  with tarfile.open(fileobj=buffer,mode='w:gz')as archive:
   for name,(content,mode)in files.items():
    info=tarfile.TarInfo('root/'+name);info.size=len(content);info.mode=mode;archive.addfile(info,io.BytesIO(content))
  buffer.seek(0);return tarfile.open(fileobj=buffer,mode='r:gz')
 def test_consistent_finite_archive_only(self):
  with self.fixture()as archive:self.assertEqual(len(archive_members(archive,'root','x86_64-unknown-linux-gnu')),13)
 def test_mutation_extra_and_nonexec(self):
  changes=[lambda f:f.update({'runtime/linux-sandbox/helper':(b'y'*128,0o755)}),lambda f:f.update({'runtime/linux-sandbox/extra':(b'x',0o644)}),lambda f:f.update({'runtime/linux-sandbox/network/usr/bin/slirp4netns':(b'x'*128,0o644)})]
  for change in changes:
   with self.fixture(change)as archive:
    with self.assertRaises(ValueError):archive_members(archive,'root','x86_64-unknown-linux-gnu')
 def test_final_archive_detects_normal_daemon_replacement(self):
  with tempfile.TemporaryDirectory()as directory:
   path=Path(directory)/'archive.tar.gz'
   with tarfile.open(path,'w:gz')as archive:
    member=tarfile.TarInfo('root/gentd');member.size=3;archive.addfile(member,io.BytesIO(b'bad'))
   expected=Components([],{'gentd':hashlib.sha256(b'good').hexdigest()})
   with self.assertRaises(ValueError):verify_archive(path,'root',expected)
 def test_target_mismatch(self):
  with self.fixture()as archive:
   with self.assertRaises(ValueError):archive_members(archive,'root','aarch64-unknown-linux-gnu')
 def test_candidate_verifier_rejected_before_execution(self):
  with tempfile.TemporaryDirectory()as d:
   root=Path(d).resolve();p=root/'gentd';p.write_bytes(b'x'*128)
   with self.assertRaises(ValueError):trusted_verifier(p,'a'*64,root)
 def test_service_is_opt_in_and_uses_real_default_data_policy(self):
  text=render_installed('1.2.3',254)
  self.assertIn('/usr/lib/gent/1.2.3/gentd',text);self.assertNotIn('--data-dir',text);self.assertNotIn('ExecStartPre',text)
  self.assertIn('DelegateSubgroup=controller',text)
  with self.assertRaises(ValueError):render_installed('../escape',254)
  with self.assertRaises(ValueError):render_installed('1.2.3',253)

class CiInputTests(unittest.TestCase):
 def module(self):
  spec=importlib.util.spec_from_file_location('ci',Path(__file__).with_name('package-linux-ci.py'));m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
 def test_copy_digest_size_and_fifo_fail_closed(self):
  module=self.module()
  with tempfile.TemporaryDirectory()as directory:
   root=Path(directory);source=root/'source';source.write_bytes(b'x'*128)
   with self.assertRaises(ValueError):module.copy_verified(source,root/'bad-digest','a'*64,256)
   with self.assertRaises(ValueError):module.copy_verified(source,root/'bad-size',hashlib.sha256(source.read_bytes()).hexdigest(),64)
   module.copy_verified(source,root/'good',hashlib.sha256(source.read_bytes()).hexdigest(),256)
   self.assertEqual((root/'good').read_bytes(),source.read_bytes())
   if hasattr(os,'mkfifo'):
    fifo=root/'fifo';os.mkfifo(fifo)
    with self.assertRaises(ValueError):module.copy_verified(fifo,root/'fifo-copy','a'*64,256)
 def test_publisher_never_overwrites_existing_output(self):
  module=self.module()
  with tempfile.TemporaryDirectory()as directory:
   root=Path(directory);source=root/'source';source.mkdir();(source/'artifact').write_bytes(b'new')
   destination=root/'destination';destination.mkdir();victim=root/'victim';victim.write_bytes(b'old')
   (destination/'artifact').symlink_to(victim)
   with self.assertRaises(FileExistsError):module.publish_outputs(source,destination)
   self.assertEqual(victim.read_bytes(),b'old')
 def test_unpack_denies_links_escape_and_duplicates(self):
  module=self.module()
  for variant in ('link','escape','duplicate'):
   with tempfile.TemporaryDirectory()as directory:
    root=Path(directory);archive=root/'bundle.tar';out=root/'out';out.mkdir()
    with tarfile.open(archive,'w')as bundle:
     member=tarfile.TarInfo('runtime/item'if variant!='escape'else'../escape')
     if variant=='link':member.type=tarfile.SYMTYPE;member.linkname='/etc/passwd';bundle.addfile(member)
     else:
      member.size=1;bundle.addfile(member,io.BytesIO(b'x'))
      if variant=='duplicate':bundle.addfile(member,io.BytesIO(b'y'))
    with self.assertRaises((ValueError,FileExistsError)):module.unpack(archive,out)

unittest.main()
