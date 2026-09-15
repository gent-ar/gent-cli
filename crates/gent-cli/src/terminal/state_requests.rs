use gent_types::{
    AgentChatProvider, AgentChatSelection, AgentChatSession, ContextPolicy, ConversationListItem,
    PermissionDecisionResponse, PermissionMode, PromptTemplateVariable,
};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum UiRequest {
    Create {
        selection: Option<AgentChatSelection>,
        session_id: Option<gent_types::AgentChatSessionId>,
    },
    Send {
        conversation_id: String,
        text: String,
        attachments: Vec<PathBuf>,
    },
    Queue {
        conversation_id: String,
        text: String,
        attachments: Vec<PathBuf>,
    },
    InvokeCommand {
        conversation_id: Option<String>,
        name: String,
        arguments: String,
        session_id: Option<gent_types::AgentChatSessionId>,
    },
    RunAutomation {
        automation_id: String,
        conversation_id: String,
    },
    Switch {
        conversation_id: String,
        parent_run_id: String,
        selection: AgentChatSelection,
        context_policy: ContextPolicy,
    },
    Permission {
        response: PermissionDecisionResponse,
    },
    SetPermissionMode {
        conversation_id: String,
        workspace_id: String,
        mode: PermissionMode,
        bypass_consent: bool,
    },
    Interrupt {
        conversation_id: String,
        run_id: String,
    },
    SteerQueued {
        conversation_id: String,
        message_ids: Vec<String>,
    },
    CancelQueued {
        conversation_id: String,
        message_id: String,
    },
    ContinueFromHistory {
        conversation_id: String,
        message_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UiRequestResult {
    pub(crate) conversation: ConversationListItem,
    pub(crate) parent_run_id: Option<String>,
    pub(crate) notice: String,
    pub(crate) permission_mode: Option<PermissionMode>,
    pub(crate) session: Option<AgentChatSession>,
    pub(crate) awaiting_turn: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum UiEffect {
    Continue,
    Quit,
    Request(UiRequest),
    RenderTemplate {
        template_id: String,
        variables: Vec<PromptTemplateVariable>,
    },
    Refresh(String),
    ListDocuments {
        workspace_id: String,
        attach_id: Option<String>,
    },
    ListTemplates,
    CreateSession(AgentChatSession),
    Login(AgentChatProvider),
}
