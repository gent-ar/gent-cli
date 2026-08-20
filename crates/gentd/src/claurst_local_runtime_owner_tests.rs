use std::{cell::RefCell, collections::VecDeque, path::PathBuf, rc::Rc};

use crate::{
    claurst_local_runtime::{
        ClaurstLocalRuntimePlan, ClaurstLocalRuntimeRequest, LocalProcessLaunch,
    },
    claurst_local_runtime_owner::{
        ClaurstLocalRuntimeOwner, LlamaServerReadiness, LocalRuntimeLauncher, LocalRuntimeProcess,
        PrivateSettingsStore,
    },
    local_model_catalog::LocalModelCatalog,
};

#[derive(Clone, Default)]
struct Events(Rc<RefCell<Vec<String>>>);
struct Store(Events);
impl PrivateSettingsStore for Store {
    fn materialize(&self, path: &std::path::Path, contents: &str) -> Result<(), String> {
        self.0
            .0
            .borrow_mut()
            .push(format!("settings:{}:{contents}", path.display()));
        Ok(())
    }
}
struct Process {
    name: String,
    events: Events,
}
impl LocalRuntimeProcess for Process {
    fn shutdown(&mut self) -> Result<(), String> {
        self.events
            .0
            .borrow_mut()
            .push(format!("stop:{}", self.name));
        Ok(())
    }
}
struct Launcher {
    events: Events,
    outcomes: RefCell<VecDeque<Result<String, String>>>,
}
impl LocalRuntimeLauncher for Launcher {
    type Process = Process;
    fn launch(&self, launch: &LocalProcessLaunch) -> Result<Process, String> {
        let name = self.outcomes.borrow_mut().pop_front().unwrap();
        self.events
            .0
            .borrow_mut()
            .push(format!("launch:{}", launch.arguments.join(" ")));
        name.map(|name| Process {
            name,
            events: self.events.clone(),
        })
    }
}
struct Ready {
    events: Events,
    outcome: Result<(), String>,
}
impl LlamaServerReadiness for Ready {
    fn wait_ready(&self, server_url: &str) -> Result<(), String> {
        self.events
            .0
            .borrow_mut()
            .push(format!("ready:{server_url}"));
        self.outcome.clone()
    }
}
fn plan() -> ClaurstLocalRuntimePlan {
    let catalog = LocalModelCatalog::shipped().unwrap();
    ClaurstLocalRuntimePlan::build(
        ClaurstLocalRuntimeRequest {
            claurst_executable: PathBuf::from("/opt/gent/bin/claurst"),
            llama_server_executable: PathBuf::from("/opt/gent/bin/llama-server"),
            model_path: PathBuf::from("/opt/gent/models/model.gguf"),
            claurst_home: PathBuf::from("/opt/gent/claurst"),
            port: 18_080,
        },
        catalog.models().first().unwrap(),
    )
    .unwrap()
}
fn owner(
    events: Events,
    launches: Vec<Result<&str, &str>>,
    ready: Result<(), &str>,
) -> ClaurstLocalRuntimeOwner<Store, Launcher, Ready> {
    ClaurstLocalRuntimeOwner::new(
        Store(events.clone()),
        Launcher {
            events: events.clone(),
            outcomes: RefCell::new(
                launches
                    .into_iter()
                    .map(|outcome| outcome.map(str::to_owned).map_err(str::to_owned))
                    .collect(),
            ),
        },
        Ready {
            events,
            outcome: ready.map_err(str::to_owned),
        },
    )
}

#[test]
fn materializes_then_starts_llama_waits_and_starts_acp_before_orderly_shutdown() {
    let events = Events::default();
    let mut owner = owner(events.clone(), vec![Ok("llama"), Ok("acp")], Ok(()));
    owner.start(&plan()).unwrap();
    owner.shutdown().unwrap();
    owner.shutdown().unwrap();
    let events = events.0.borrow();
    assert!(events[0].starts_with("settings:/opt/gent/claurst/settings.json:"));
    assert_eq!(
        &events[1..],
        [
            "launch:-m /opt/gent/models/model.gguf --host 127.0.0.1 --port 18080 --jinja --ctx-size 32768 --parallel 1",
            "ready:http://127.0.0.1:18080",
            "launch:acp",
            "stop:acp",
            "stop:llama",
        ]
    );
}
#[test]
fn readiness_failure_stops_llama_and_never_starts_acp() {
    let events = Events::default();
    let mut owner = owner(events.clone(), vec![Ok("llama")], Err("not yet"));
    assert!(owner.start(&plan()).unwrap_err().contains("not ready"));
    assert_eq!(
        events.0.borrow()[1..],
        [
            "launch:-m /opt/gent/models/model.gguf --host 127.0.0.1 --port 18080 --jinja --ctx-size 32768 --parallel 1",
            "ready:http://127.0.0.1:18080",
            "stop:llama",
        ]
    );
}
#[test]
fn acp_launch_failure_stops_the_ready_llama_server() {
    let events = Events::default();
    let mut owner = owner(events.clone(), vec![Ok("llama"), Err("missing")], Ok(()));
    assert!(owner.start(&plan()).unwrap_err().contains("Claurst ACP"));
    assert_eq!(events.0.borrow().last(), Some(&"stop:llama".to_string()));
}
