use super::action_observer::{self, Actions};
use gent_drivers::codex_runner::{CodexAppServerRunner, CodexRunnerEffect};
use gent_drivers::public_protocol::PublicWireFact;
use gent_drivers::{ProcessLauncher, SystemLauncher};
use gent_ports::PolicyLedger;
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, NormalizedLifecycleSignal, NormalizedProviderEvent, SandboxPermissionBinding,
    TurnPhase,
};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Turn {
    pub(super) output: String,
    pub(super) actions: Actions,
    pub(super) completed: bool,
    pub(super) failure: bool,
}

pub(super) fn drive<L: ProcessLauncher>(
    runner: &mut CodexAppServerRunner<L, L::Process>,
    ledger: &SqliteLedger,
    binding: &SandboxPermissionBinding,
    run: &str,
) -> Turn {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut turn = Turn::default();
    while Instant::now() < deadline && !turn.completed && !turn.failure {
        let Some(effects) = runner.poll(run).unwrap() else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        for effect in effects {
            if let Some(diagnosis) = action_observer::terminal_diagnostic(&effect) {
                println!("smoke-terminal {diagnosis}");
            }
            if let CodexRunnerEffect::Fact(ref fact) = effect {
                turn.actions.observe(fact);
            }
            observe(runner, ledger, binding, effect, &mut turn);
        }
    }
    turn
}

fn observe<L: ProcessLauncher>(
    runner: &mut CodexAppServerRunner<L, L::Process>,
    ledger: &SqliteLedger,
    binding: &SandboxPermissionBinding,
    effect: CodexRunnerEffect,
    turn: &mut Turn,
) {
    match effect {
        CodexRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output {
            text,
            is_partial: false,
        })) => turn.output.push_str(&text),
        CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        )) => turn.completed = true,
        CodexRunnerEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ProviderFailure { classification, .. },
        )) => {
            turn.failure = true;
            println!("provider failure classification={classification:?}; no retry");
        }
        CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase:
                    TurnPhase::Failed | TurnPhase::Dead | TurnPhase::Cancelled | TurnPhase::Interrupted,
            },
        ))
        | CodexRunnerEffect::Exited { .. } => turn.failure = true,
        CodexRunnerEffect::ControlRequest(request) => {
            if let Err(reason) = super::smoke_controls::respond(runner, ledger, binding, request) {
                println!("smoke-control-blocked reason={reason}");
                turn.failure = true;
            }
        }
        _ => {}
    }
}

pub(super) fn verify(
    ledger: &SqliteLedger,
    binding: &SandboxPermissionBinding,
    root: &Path,
    workspace: &Path,
    marker: &str,
    command_edit: bool,
    turn: &Turn,
) {
    let mut stale = binding.policy().clone();
    stale.revision += 1;
    stale.policy_id = "smoke-after-turn".into();
    ledger.save_policy(&stale).unwrap();
    let stale_launcher = crate::sandbox_provider_launcher::SandboxProviderLauncher::new(
        SystemLauncher::new(1024),
        (*ledger).clone(),
        Coordinator::new((*ledger).clone(), CapabilitySet::default()),
        root.join("unused-state"),
        1024,
    );
    assert!(stale_launcher.validate_permission_binding(binding).is_err());
    let actions = &turn.actions;
    if command_edit {
        println!(
            "autonomous-actions command={} edit={} failed_tools={} shell_diagnostics={:?}",
            actions.command_printed(marker),
            actions.edit_completed,
            actions.failed_tools,
            actions.diagnostics
        );
    }
    assert!(!turn.failure);
    assert!(turn.completed, "one prompt did not complete; no retry");
    assert_eq!(turn.output.trim().lines().last(), Some(marker));
    if command_edit {
        assert!(
            actions.command_printed(marker),
            "no completed command printed exact marker"
        );
        assert!(actions.edit_completed, "no completed file-change activity");
        assert_eq!(
            std::fs::read(workspace.join("proof.txt")).unwrap(),
            format!("{marker}\n").as_bytes()
        );
        assert_eq!(std::fs::read_dir(workspace).unwrap().count(), 1);
    } else {
        assert_eq!(
            std::fs::read_dir(workspace).unwrap().count(),
            0,
            "no-tool workspace must stay empty"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
