use super::{
    mcp::{Error, Peer},
    mcp_stdio::{Spec, StdioPeer},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
#[test]
fn host_mcp_stdio_rechecks_captured_scope_immediately_before_actual_write() {
    let root = Arc::new(tempfile::tempdir().unwrap());
    let script = root.path().join("server.ps1");
    let effect = root.path().join("effect.txt");
    std::fs::write(&script, "while ($null -ne ($line = [Console]::In.ReadLine())) { $m = ConvertFrom-Json $line; Set-Content effect.txt must-not-happen; [Console]::Out.WriteLine((ConvertTo-Json -Compress @{jsonrpc='2.0'; id=$m.id; result=@{}})); [Console]::Out.Flush() }").unwrap();
    let powershell = powershell();
    let peer = StdioPeer::spawn(Spec {
        launch_guard: Arc::new(|| true),
        authority: root.clone(),
        startup: None,
        program: powershell.clone(),
        args: script_arguments(&script),
        env: BTreeMap::new(),
        cwd: root.path().into(),
        pins: vec![
            std::fs::File::open(powershell).unwrap(),
            std::fs::File::open(script).unwrap(),
        ],
        secrets: vec![],
        timeout: Duration::from_secs(5),
    })
    .unwrap();
    let barrier = Arc::new((Mutex::new((false, false, "T1".to_owned())), Condvar::new()));
    let for_writer = barrier.clone();
    let guard = Arc::new(move || {
        let (lock, wake) = &*for_writer;
        let mut state = lock.lock().unwrap();
        state.0 = true;
        wake.notify_all();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !state.1 && Instant::now() < deadline {
            state = wake
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap()
                .0;
        }
        state.1 && state.2 == "T1"
    });
    let owned = peer.clone();
    let call = std::thread::spawn(move || {
        owned.request_bound(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"write"}}),guard)
    });
    let (lock, wake) = &*barrier;
    let mut state = lock.lock().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !state.0 && Instant::now() < deadline {
        state = wake
            .wait_timeout(state, Duration::from_millis(20))
            .unwrap()
            .0;
    }
    assert!(state.0);
    state.2 = "T2".into();
    state.1 = true;
    wake.notify_all();
    drop(state);
    assert!(matches!(call.join().unwrap(), Err(Error::Unprovable)));
    assert!(peer.wait_settled(Duration::from_secs(5)));
    assert!(!effect.exists());
}

pub(crate) fn powershell() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe")
}

pub(crate) fn script_arguments(script: &std::path::Path) -> Vec<String> {
    [
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ]
    .into_iter()
    .map(String::from)
    .chain([script.to_string_lossy().into_owned()])
    .collect()
}
