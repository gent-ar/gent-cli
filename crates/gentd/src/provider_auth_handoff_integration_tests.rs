use super::{ProviderAuthPort, StandaloneProviderAuthPort};
use gent_protocol::ProviderAuthFrame;
use gent_types::{ProviderAuthHandoffMethod, ProviderAuthLifecycle, ProviderAuthProvider};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn fixture(directory: &Path) -> StandaloneProviderAuthPort {
    let executable = directory.join("codex-fixture");
    fs::write(&executable, r"#!/usr/bin/env python3
import json,sys,threading,time
from pathlib import Path
root=Path(__file__).parent
if sys.argv[1:] == ['--version']:
 print('codex-cli 0.145.0');sys.exit(0)
if sys.argv[1:] == ['login','status']:
 print('Not logged in',file=sys.stderr);sys.exit(1)
def emit(value):
 print(json.dumps(value),flush=True)
stop=threading.Event()
def complete():
 while not stop.wait(.01):
  if (root/'complete').exists():
   emit({'method':'account/login/completed','params':{'loginId':'native-login','success':True}})
   return
for line in sys.stdin:
 value=json.loads(line);method=value.get('method')
 if method=='initialize':emit({'id':value['id'],'result':{'userAgent':'fixture'}})
 elif method=='account/login/start':
  emit({'id':value['id'],'result':{'type':'chatgptDeviceCode','loginId':'native-login','verificationUrl':'https://auth.openai.com/codex/device','userCode':'ABCD-EFGH','deviceSecret':'PRIVATE-DEVICE-SECRET'}})
  threading.Thread(target=complete,daemon=True).start()
 elif method=='account/login/cancel':
  stop.set();emit({'id':value['id'],'result':{'status':'canceled'}})
stop.set()
").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    StandaloneProviderAuthPort::for_test(
        crate::provider_executables::ProviderExecutables::explicit(None, Some(executable)),
    )
}

fn begin(port: &StandaloneProviderAuthPort) -> String {
    let reply = port
        .exchange(ProviderAuthFrame::HandoffLoginRequest {
            request_id: "begin".into(),
            provider: ProviderAuthProvider::Codex,
        })
        .unwrap();
    let ProviderAuthFrame::HandoffState { state, .. } = reply else {
        panic!("handoff response expected")
    };
    let challenge = state.challenge.unwrap().challenge_id;
    assert!(
        state
            .methods
            .contains(&ProviderAuthHandoffMethod::DeviceVerification)
    );
    port.exchange(ProviderAuthFrame::HandoffSelectMethod {
        request_id: "select".into(),
        challenge_id: challenge.clone(),
        method: ProviderAuthHandoffMethod::DeviceVerification,
    })
    .unwrap();
    challenge
}

fn offered(port: &StandaloneProviderAuthPort) -> gent_types::ProviderAuthHandoffState {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let reply = port
            .exchange(ProviderAuthFrame::HandoffStatusRequest {
                request_id: "reopen".into(),
                provider: ProviderAuthProvider::Codex,
            })
            .unwrap();
        let ProviderAuthFrame::HandoffState { state, .. } = reply else {
            panic!("handoff response expected")
        };
        if state.handoff.is_some() {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "device instructions were not produced"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn reopened_device_handoff_is_exactly_bound_legacy_safe_and_cancelable() {
    let directory = tempfile::tempdir().unwrap();
    let port = fixture(directory.path());
    let challenge = begin(&port);
    let first = offered(&port);
    let second = offered(&port);
    let control = port.lifecycle_control.as_ref().unwrap();
    assert!(
        control.quiesce_if_idle(|| Ok(())).is_err(),
        "live auth child must keep maintenance busy"
    );
    assert_eq!(first, second);
    assert_eq!(
        first.status.challenge_id.as_deref(),
        Some(challenge.as_str())
    );
    let public = serde_json::to_string(&first).unwrap();
    assert!(!public.contains("PRIVATE-DEVICE-SECRET"));
    assert!(!public.contains("native-login"));
    let old = port
        .exchange(ProviderAuthFrame::StatusRequest {
            request_id: "legacy".into(),
            provider: ProviderAuthProvider::Codex,
        })
        .unwrap();
    let legacy = serde_json::to_string(&old).unwrap();
    assert!(!legacy.contains("ABCD-EFGH"));
    assert!(!legacy.contains("verificationUrl"));
    assert!(
        port.exchange(ProviderAuthFrame::Cancel {
            request_id: "stale".into(),
            challenge_id: "other".into()
        })
        .is_err()
    );
    let canceled = port
        .exchange(ProviderAuthFrame::Cancel {
            request_id: "cancel".into(),
            challenge_id: challenge,
        })
        .unwrap();
    assert!(
        matches!(canceled,ProviderAuthFrame::Status{status,..} if status.lifecycle==ProviderAuthLifecycle::Cancelled)
    );
    let after = port
        .exchange(ProviderAuthFrame::HandoffStatusRequest {
            request_id: "after".into(),
            provider: ProviderAuthProvider::Codex,
        })
        .unwrap();
    assert!(!serde_json::to_string(&after).unwrap().contains("ABCD-EFGH"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.quiesce_if_idle(|| Ok(())).is_err() {
        assert!(
            Instant::now() < deadline,
            "cancelled child did not release admission after exit"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn native_matching_completion_settles_without_projecting_private_native_values() {
    let directory = tempfile::tempdir().unwrap();
    let port = fixture(directory.path());
    begin(&port);
    offered(&port);
    fs::write(directory.path().join("complete"), b"fixture").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut sessions = super::recover(&port.sessions);
        super::refresh(&mut sessions[0], Duration::from_millis(100));
        if sessions[0].status().lifecycle == ProviderAuthLifecycle::Authenticated {
            assert!(sessions[0].state.challenge.is_none());
            assert!(sessions[0].device_login.is_none());
            break;
        }
        drop(sessions);
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
}
