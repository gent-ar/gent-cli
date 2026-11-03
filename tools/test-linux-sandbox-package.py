import importlib.util,json,tempfile,unittest,hashlib
from pathlib import Path
spec=importlib.util.spec_from_file_location('candidate',Path(__file__).parent/'linux_sandbox_package.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
class Tests(unittest.TestCase):
 def test_relative(self):
  for s in ('../x','/x','a//b','a/./b','a\\b'):
   with self.assertRaises(ValueError):m.relative(s)
 def test_complete_layout(self):
  with tempfile.TemporaryDirectory() as d:
   root=Path(d).resolve(); resources={'platform':'x86_64-unknown-linux-gnu','helper':'runtime/helper','node':'runtime/node/bin/node','gent':'gent','network':'runtime/network','mcpEntries':{k:'mcp/'+k+'.js' for k in m.ROLES}}
   for f in [resources[k] for k in ('helper','node','gent')]+list(resources['mcpEntries'].values()):
    p=root/f;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b'x'*128)
   p=root/'runtime/network/manifest.json';p.parent.mkdir(parents=True)
   entries=[]
   for f in ('usr/bin/slirp4netns','usr/sbin/iptables','usr/sbin/ip6tables','usr/sbin/iptables-restore','usr/sbin/ip6tables-restore'):
    file=p.parent/f;file.parent.mkdir(parents=True,exist_ok=True);file.write_bytes(b'x'*128);entries.append({'path':f,'sha256':hashlib.sha256(file.read_bytes()).hexdigest()})
   p.write_text(json.dumps({'schema':1,'files':entries}))
   result=m.prepare(root,resources,'a'*64,123)
   self.assertEqual(list(result['mcpEntries']),list(m.ROLES));self.assertEqual(result['node']['path'],'runtime/node/bin/node')
   (p.parent/'unmanifested').write_text('extra')
   with self.assertRaises(ValueError):m.prepare(root,resources,'a'*64,123)
   (p.parent/'unmanifested').unlink()
   (p.parent/entries[0]['path']).write_bytes(b'y'*128)
   with self.assertRaises(ValueError):m.prepare(root,resources,'a'*64,123)
   resources['mcpEntries'].pop('gent-browser')
   with self.assertRaises(ValueError):m.prepare(root,resources,'a'*64,123)
 def test_pin_size_and_mutation(self):
  with tempfile.TemporaryDirectory() as d:
   root=Path(d).resolve();p=root/'image';p.write_bytes(b'x'*63)
   with self.assertRaises(ValueError):m.pin(root,'image',1024)
   p.write_bytes(b'x'*128);a=m.pin(root,'image',1024);p.write_bytes(b'y'*128);self.assertNotEqual(a['sha256'],m.pin(root,'image',1024)['sha256'])
class RaceTests(unittest.TestCase):
 def test_real_path_replacement_during_hash(self):
  with tempfile.TemporaryDirectory() as d:
   root=Path(d).resolve();p=root/'image';p.write_bytes(b'x'*131072)
   real=m.hashlib.sha256
   class Changed:
    def __init__(self):self.inner=real();self.changed=False
    def update(self,data):
     self.inner.update(data)
     if not self.changed:
      replacement=root/'next';replacement.write_bytes(b'y'*131072);replacement.replace(p);self.changed=True
    def hexdigest(self):return self.inner.hexdigest()
   m.hashlib.sha256=Changed
   try:
    with self.assertRaises(ValueError):m.pin(root,'image',1<<20)
   finally:m.hashlib.sha256=real
 def test_fifo_and_bounded_metadata(self):
  import os
  with tempfile.TemporaryDirectory() as d:
   root=Path(d).resolve();fifo=root/'fifo';os.mkfifo(fifo)
   with self.assertRaises(ValueError):m.bounded_read(fifo,128)
   f=root/'large';f.write_bytes(b'x'*129)
   with self.assertRaises(ValueError):m.bounded_read(f,128)
class ServiceTests(unittest.TestCase):
 def test_render_is_explicit_and_escapes(self):
  spec=importlib.util.spec_from_file_location('service',Path(__file__).parent/'linux_service_unit.py')
  service=importlib.util.module_from_spec(spec);spec.loader.exec_module(service)
  self.assertEqual(service.argument('a%$"'), '\"a%%$$\\\"\"')
  with self.assertRaises(ValueError):service.argument('a\nExecStart=bad')
  with tempfile.TemporaryDirectory() as d:
   root=Path(d).resolve();(root/'gentd').write_bytes(b'candidate');data=root/'data';data.mkdir()
   text=service.render(root,data,254)
   self.assertIn('DelegateSubgroup=controller',text);self.assertIn('--prepare-linux-service-delegation',text)
   self.assertNotIn('sudo',text)
   with self.assertRaises(ValueError):service.render(root,data,253)
unittest.main()
