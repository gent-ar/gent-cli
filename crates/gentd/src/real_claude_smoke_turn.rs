use gent_drivers::claude_control::{ClaudePermissionBehavior, ClaudePermissionRequest};
use gent_drivers::claude_runner::{ClaudeRunnerEffect, ClaudeStreamRunner};
use gent_drivers::public_protocol::PublicWireFact;
use gent_drivers::{ProcessLauncher, ProviderProcess};
use gent_ports::PolicyLedger;
use gent_store::SqliteLedger;
use gent_types::{
    HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent, PermissionRequest, PolicyScope,
    SandboxPermissionBinding, ToolPhase, TurnPhase,
};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Turn {
    pub(super) output: String,
    pub(super) tools: BTreeSet<String>,
    pub(super) completed: bool,
    pub(super) failure: bool,
}

pub(super) struct Session<'a, L, P> {
    pub(super) runner: &'a mut ClaudeStreamRunner<L, P>,
    pub(super) ledger: &'a SqliteLedger,
    pub(super) binding: &'a SandboxPermissionBinding,
    pub(super) run: &'a str,
    pub(super) conversation: &'a str,
}

impl<L, P> Session<'_, L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    pub(super) fn drive(&mut self) -> Turn {
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut turn = Turn::default();
        while Instant::now() < deadline && !turn.completed && !turn.failure {
            let Some(effects) = self.runner.poll(self.run).unwrap() else {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            };
            for effect in effects {
                self.observe(effect, &mut turn);
            }
        }
        turn
    }

    fn observe(&mut self, effect: ClaudeRunnerEffect, turn: &mut Turn) {
        match effect {
            ClaudeRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output {
                text,
                ..
            })) => turn.output.push_str(&text),
            ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ToolActivity { activity },
            )) if activity.phase == ToolPhase::Completed => {
                turn.tools.insert(activity.tool_name);
            }
            ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::Ready,
                },
            )) => turn.completed = true,
            ClaudeRunnerEffect::Fact(PublicWireFact::Event(
                NormalizedProviderEvent::ProviderFailure { classification, .. },
            )) => {
                println!("terminal provider-failure={classification:?}");
                turn.failure = true;
            }
            ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase {
                    phase:
                        TurnPhase::Failed
                        | TurnPhase::Dead
                        | TurnPhase::Cancelled
                        | TurnPhase::Interrupted,
                },
            )) => turn.failure = true,
            ClaudeRunnerEffect::PermissionRequest(request) => {
                if !self.respond(&request) {
                    turn.failure = true;
                }
            }
            ClaudeRunnerEffect::Exited { code } => {
                println!("terminal exit={code:?}");
                turn.failure = true;
            }
            _ => {}
        }
    }

    fn respond(&mut self, request: &ClaudePermissionRequest) -> bool {
        let current = self
            .ledger
            .current_policy("smoke-workspace", PolicyScope::ProviderPermissions)
            .unwrap()
            .unwrap();
        assert_eq!(&current, self.binding.policy());
        let normalized = PermissionRequest::new(
            request.tool_name.clone(),
            crate::permission_category::for_tool(&request.tool_name),
            request.operation_preview.clone(),
            request.child_id.clone(),
        );
        let decision = crate::permission_preflight::evaluate_callback(
            &current,
            &normalized,
            self.run,
            self.conversation,
            HostEpoch(1),
            self.runner.sandbox_evidence(self.run),
            false,
        );
        let tool = match request.tool_name.as_str() {
            "Read" => "Read",
            "Write" => "Write",
            _ => "other",
        };
        println!("policy-callback tool={tool} decision={decision:?}");
        let allowed = decision == crate::permission_preflight::PermissionPreflight::Allow;
        let behavior = if allowed {
            ClaudePermissionBehavior::Allow
        } else {
            ClaudePermissionBehavior::Deny
        };
        self.runner
            .respond_permission(self.run, &request.request_id, behavior, false)
            .unwrap();
        allowed
    }
}
