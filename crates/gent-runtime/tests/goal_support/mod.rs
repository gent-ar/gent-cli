use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, ConversationActivityLedger,
    ConversationLedger, Ledger,
};
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationRequest, AgentChatConversationResult,
    AgentChatConversationService, GoalAuthority, GoalContinuationAdmission, GoalContinuationWake,
    GoalPursuitService, GoalPursuitTick, GoalResult, GoalService,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider, AgentChatRequestId,
    AgentChatSelection, ConversationActivityFact, ConversationActivityScope, DurableTurnPhase,
    GoalRecord, HostEpoch, ReceiptId, ToolActivity, ToolPhase, WorkspaceRecord,
};

pub struct Harness {
    pub directory: tempfile::TempDir,
    pub ledger: SqliteLedger,
    pub epoch: HostEpoch,
    pub conversation: AgentChatConversationId,
    pub goals: GoalService<SqliteLedger>,
    pub pursuit: GoalPursuitService<SqliteLedger>,
    prompts: u32,
    cursor: u64,
}

pub struct Releasing(pub SqliteLedger, pub HostEpoch);

impl GoalContinuationAdmission for Releasing {
    fn admit(&mut self, wake: &GoalContinuationWake) -> Result<(), String> {
        self.0
            .release_agent_chat_prompt_after_readiness(&wake.message_id, &wake.run_id, self.1)
            .map_err(|error| error.to_string())
    }
}

pub struct Refusing;

impl GoalContinuationAdmission for Refusing {
    fn admit(&mut self, _: &GoalContinuationWake) -> Result<(), String> {
        Err("provider readiness was not proven".into())
    }
}

pub struct Unreleased;

impl GoalContinuationAdmission for Unreleased {
    fn admit(&mut self, _: &GoalContinuationWake) -> Result<(), String> {
        Ok(())
    }
}

impl Harness {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
        let created = AgentChatConversationService::new(
            ledger.clone(),
            AgentChatConversationAuthority::Approved,
        )
        .create(&AgentChatConversationRequest {
            request_id: AgentChatRequestId("conversation".into()),
            receipt_id: ReceiptId("conversation".into()),
            host_epoch: HostEpoch(1),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "haiku".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Agent,
            },
            workspace: WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        })
        .unwrap();
        let AgentChatConversationResult::Created(created) = created else {
            panic!("approved setup creates a conversation");
        };
        Self {
            goals: GoalService::new(ledger.clone(), GoalAuthority::Approved),
            pursuit: GoalPursuitService::new(ledger.clone()),
            conversation: created.conversation_id,
            directory,
            ledger,
            epoch: HostEpoch(1),
            prompts: 0,
            cursor: 10_000,
        }
    }

    pub fn restart(&mut self) {
        let previous = self.epoch;
        self.ledger.close_ingress(previous).unwrap();
        self.epoch = self.ledger.fence_and_open(previous).unwrap().epoch;
        self.ledger = SqliteLedger::open(self.directory.path().join("gent.db")).unwrap();
        self.ledger
            .recover_agent_chat_prompt_dispatches(self.epoch)
            .unwrap();
        self.goals = GoalService::new(self.ledger.clone(), GoalAuthority::Approved);
        self.pursuit = GoalPursuitService::new(self.ledger.clone());
    }

    pub fn set(&self, objective: &str, budget: Option<u64>, now: u64) -> GoalRecord {
        let GoalResult::Goal(Some(goal)) = self
            .goals
            .set(
                "set-request",
                &self.conversation,
                objective.into(),
                budget,
                self.epoch,
                now,
            )
            .unwrap()
        else {
            panic!("goal was not set");
        };
        goal
    }

    pub fn current(&self) -> GoalRecord {
        let GoalResult::Goal(Some(goal)) = self.goals.current(&self.conversation).unwrap() else {
            panic!("goal is missing");
        };
        goal
    }

    pub fn tick(&self, now: u64) -> GoalPursuitTick {
        self.pursuit
            .tick(
                self.epoch,
                now,
                &mut Releasing(self.ledger.clone(), self.epoch),
            )
            .unwrap()
    }

    pub fn user_prompt(&mut self, disposition: AgentChatPromptDisposition) -> AgentChatPromptSaved {
        self.prompts += 1;
        let saved = self
            .ledger
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId(format!("user-{}", self.prompts)),
                receipt_id: ReceiptId(format!("user-{}", self.prompts)),
                host_epoch: self.epoch,
                conversation_id: self.conversation.clone(),
                disposition,
                text: "user work".into(),
                attachment_ids: vec![],
                tool_source_ids: vec![],
            })
            .unwrap();
        self.ledger
            .release_agent_chat_prompt_after_readiness(
                &saved.message.message_id,
                &saved.run_id,
                self.epoch,
            )
            .unwrap();
        saved
    }

    pub fn switch(&self, provider: AgentChatProvider, model: &str) -> gent_types::AgentChatRunId {
        let parent_run_id = self
            .ledger
            .list_conversation_runs(&self.conversation.0)
            .unwrap()
            .last()
            .unwrap()
            .run_id
            .clone();
        let result = gent_runtime::AgentChatSelectionSwitchService::new(
            self.ledger.clone(),
            gent_runtime::AgentChatSelectionSwitchAuthority::Approved,
        )
        .switch(&gent_runtime::AgentChatSelectionSwitchRequest {
            request_id: AgentChatRequestId(format!("switch-{model}")),
            receipt_id: ReceiptId(format!("switch-{model}")),
            host_epoch: self.epoch,
            conversation_id: self.conversation.clone(),
            parent_run_id: gent_types::AgentChatRunId(parent_run_id),
            selection: AgentChatSelection {
                provider,
                model: model.into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Agent,
            },
            context_policy: gent_types::ContextPolicy::Preserve,
        })
        .unwrap();
        let gent_runtime::AgentChatSelectionSwitchResult::Switched(switched) = result else {
            panic!("approved switch creates a child run");
        };
        switched.run_id
    }

    pub fn start_next(&self) -> AgentChatPromptSaved {
        self.start_next_for(AgentChatProvider::Claude)
    }

    pub fn start_next_for(&self, provider: AgentChatProvider) -> AgentChatPromptSaved {
        let saved = self
            .ledger
            .claim_agent_chat_prompt_dispatch("provider-host", self.epoch, provider)
            .unwrap()
            .expect("a claimable prompt");
        let message = &saved.message.message_id;
        self.ledger
            .begin_agent_chat_prompt_launch(message, "provider-host", self.epoch)
            .unwrap();
        self.ledger
            .confirm_agent_chat_prompt_started(message, "provider-host", self.epoch)
            .unwrap();
        saved
    }

    pub fn work(&mut self, saved: &AgentChatPromptSaved, tokens: u64, tool: bool) {
        let mut scope = || {
            self.cursor += 1;
            ConversationActivityScope {
                conversation_id: self.conversation.0.clone(),
                run_id: saved.run_id.0.clone(),
                turn_id: saved.message.turn_id.clone(),
                host_epoch: self.epoch,
                cursor: self.cursor,
            }
        };
        let mut facts = vec![ConversationActivityFact::TokenUsage {
            scope: scope(),
            usage: gent_types::TokenUsage {
                input_tokens: tokens,
                ..gent_types::TokenUsage::default()
            },
        }];
        if tool {
            facts.push(ConversationActivityFact::ToolActivity {
                scope: scope(),
                activity: ToolActivity {
                    tool_use_id: format!("tool-{}", saved.message.turn_id),
                    tool_name: "Edit".into(),
                    phase: ToolPhase::Completed,
                    output_digest: None,
                    parent_tool_use_id: None,
                },
            });
        }
        for fact in facts {
            self.ledger.append_conversation_activity(&fact).unwrap();
        }
    }

    pub fn settle(&self, saved: &AgentChatPromptSaved, phase: DurableTurnPhase) {
        self.ledger
            .settle_agent_chat_prompt_terminal(
                &saved.message.message_id,
                "provider-host",
                self.epoch,
                phase,
            )
            .unwrap();
    }

    pub fn complete(&mut self, tokens: u64, tool: bool) -> AgentChatPromptSaved {
        let saved = self.start_next();
        self.work(&saved, tokens, tool);
        self.settle(&saved, DurableTurnPhase::Completed);
        saved
    }
}
