from pathlib import Path
import hashlib,json,os,shutil,socket,subprocess,sys,threading,time

plan=json.loads(Path(sys.argv[1]).read_text())
root=Path(plan['root'])
assert root.parent==Path('/tmp') and root.name.startswith('gent-linux-mount-fixture-')
assert len(root.name.rsplit('-',1)[1])==32 and all(c in '0123456789abcdef' for c in root.name.rsplit('-',1)[1])
assert not root.exists()
result={'kernel':os.uname().release,'uid':os.geteuid(),'bwrapVersion':subprocess.run(['/usr/bin/bwrap','--version'],capture_output=True,text=True,timeout=3).stdout.strip(),'component':'actual bubblewrap mount/network namespaces only','cases':{},'providerInvocations':0,'resourceLimitsProven':False,'nativeBackendEnabled':False}
listener=None
try:
 root.mkdir(mode=0o700)
 for name in ['work','private','runtime','hidden']:(root/name).mkdir()
 for name in ['work','runtime','hidden']:(root/name/'canary').write_text('owned-'+name)
 listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen(4);listener.settimeout(.1)
 port=listener.getsockname()[1];accepted=[];stop=threading.Event()
 def receive():
  while not stop.is_set():
   try:
    conn,_=listener.accept()
    with conn:accepted.append(conn.recv(64).decode())
   except TimeoutError:pass
 thread=threading.Thread(target=receive);thread.start()
 def barrier(marker):
  with socket.create_connection(('127.0.0.1',port),timeout=2) as conn:conn.sendall(marker.encode())
  deadline=time.monotonic()+2
  while marker not in accepted and time.monotonic()<deadline:time.sleep(.01)
  assert marker in accepted
 barrier('before')
 code=r'''
import json,os,pathlib,socket,subprocess,sys
root=pathlib.Path(sys.argv[1]);writable=sys.argv[2]=='write';port=int(sys.argv[3]);r={}
r['workspaceRead']=(root/'work/canary').read_text()=='owned-work'
r['runtimeRead']=(root/'runtime/canary').read_text()=='owned-runtime'
def write(path):
 try:path.write_text('unexpected');return True
 except OSError:return False
r['workspaceWriteMatchesMode']=write(root/'work/new')==writable
r['runtimeWriteDenied']=not write(root/'runtime/new')
r['privateWrite']=write(root/'private/new')
r['hiddenAbsent']=not (root/'hidden/canary').exists()
r['hostEtcAbsent']=not pathlib.Path('/etc/passwd').exists()
r['environmentCleared']='GENT_LINUX_CANARY' not in os.environ
r['hostParentPidHidden']=not pathlib.Path('/proc/'+sys.argv[4]).exists()
r['userNamespaceRecreationDenied']=subprocess.run(['/usr/bin/unshare','--user','--map-root-user','/usr/bin/true'],capture_output=True,timeout=3).returncode!=0
try:
 with socket.create_connection(('127.0.0.1',port),timeout=1) as conn:conn.sendall(b'escaped')
 r['hostLoopbackDenied']=False
except OSError:r['hostLoopbackDenied']=True
print(json.dumps(r,sort_keys=True))
sys.exit(0 if all(r.values()) else 1)
'''
 for key,mode in [('readOnly','read'),('readWrite','write')]:
  command=['/usr/bin/bwrap',*plan[key],'--','/usr/bin/python3','-c',code,str(root),mode,str(port),str(os.getpid())]
  child=subprocess.run(command,env={'PATH':'/usr/bin:/bin','GENT_LINUX_CANARY':'owned-canary'},capture_output=True,text=True,timeout=20)
  result['cases'][key]={'exitCode':child.returncode,'stdout':child.stdout,'stderr':child.stderr}
  assert child.returncode==0, json.dumps(result)
  assert all(json.loads(child.stdout).values())
 barrier('after');stop.set();thread.join(timeout=2);assert not thread.is_alive()
 result['receiverMarkers']=accepted
 assert accepted==['before','after']
 assert (root/'work/canary').read_text()=='owned-work'
 assert (root/'runtime/canary').read_text()=='owned-runtime'
 result['status']='passed'
except Exception as error:
 result['status']='failed';result['failureType']=type(error).__name__;result['failure']=str(error)
finally:
 if 'stop' in locals():stop.set()
 if 'thread' in locals():thread.join(timeout=2)
 if listener:listener.close()
 assert root.parent==Path('/tmp') and root.name.startswith('gent-linux-mount-fixture-')
 if root.exists():shutil.rmtree(root)
 result['ownedRootRemoved']=not root.exists()
 result['bwrapSha256']=hashlib.sha256(Path('/usr/bin/bwrap').read_bytes()).hexdigest()
 print(json.dumps(result,sort_keys=True))
sys.exit(0 if result['status']=='passed' else 1)
