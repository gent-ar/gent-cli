use clap::Subcommand;
use gent_protocol::{DependencyAction, DependencyProvider};

#[derive(Debug, Subcommand)]
pub(crate) enum ForgeCommand {
    #[command(about = "List a workspace's Forge connectors")]
    List {
        #[arg(help = "Workspace id")]
        workspace_id: String,
    },
    #[command(about = "Show one Forge connector")]
    Get {
        #[arg(help = "Workspace id")]
        workspace_id: String,
        #[arg(help = "Connector id")]
        connector_id: String,
    },
    #[command(about = "Create a Forge connector from a JSON definition")]
    Create {
        #[arg(help = "Connector definition as JSON")]
        connector: String,
    },
    #[command(about = "Enable a Forge connector")]
    Enable {
        #[arg(help = "Workspace id")]
        workspace_id: String,
        #[arg(help = "Connector id")]
        connector_id: String,
    },
    #[command(about = "Disable a Forge connector")]
    Disable {
        #[arg(help = "Workspace id")]
        workspace_id: String,
        #[arg(help = "Connector id")]
        connector_id: String,
    },
    #[command(about = "Invoke a Forge connector")]
    Invoke {
        #[arg(help = "Workspace id")]
        workspace_id: String,
        #[arg(help = "Connector id")]
        connector_id: String,
        #[arg(long, help = "Tool to invoke")]
        tool_name: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum AutomationCommand {
    #[command(about = "List a workspace's automations")]
    List {
        #[arg(help = "Workspace id")]
        workspace_id: String,
    },
    #[command(about = "Create an automation from a JSON definition")]
    Create {
        #[arg(help = "Automation definition as JSON")]
        definition: String,
    },
    #[command(about = "Run an automation now")]
    Run {
        #[arg(help = "Automation id")]
        automation_id: String,
    },
    #[command(about = "List recent runs of an automation")]
    Runs {
        #[arg(help = "Automation id")]
        automation_id: String,
        #[arg(long, default_value_t = 20, help = "Maximum runs to list")]
        limit: u16,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum SessionCommand {
    #[command(about = "List a workspace's sessions")]
    List {
        #[arg(help = "Workspace id")]
        workspace_id: String,
    },
    #[command(about = "Create a session from a JSON definition")]
    Create {
        #[arg(help = "Session definition as JSON")]
        session: String,
    },
    #[command(about = "Make a session the selected one")]
    Select {
        #[arg(help = "Session id")]
        session_id: String,
    },
    #[command(about = "Attach a conversation to a session")]
    Attach {
        #[arg(help = "Session id")]
        session_id: String,
        #[arg(help = "Conversation id")]
        conversation_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum DependencyCommand {
    #[command(about = "Show the install or update plan for a provider CLI without running it")]
    Plan {
        #[arg(help = "Action to plan")]
        action: DependencyAction,
        #[arg(help = "Provider CLI")]
        provider: DependencyProvider,
    },
    #[command(about = "Install a provider CLI after reviewing its plan")]
    Install {
        #[arg(help = "Provider CLI")]
        provider: DependencyProvider,
        #[arg(long, help = "Confirm that the reviewed install plan may run")]
        consent: bool,
        #[arg(
            long,
            help = "Idempotency key; reuse it to retry the same action safely"
        )]
        idempotency_key: Option<String>,
    },
    #[command(about = "Update a provider CLI after reviewing its plan")]
    Update {
        #[arg(help = "Provider CLI")]
        provider: DependencyProvider,
        #[arg(long, help = "Confirm that the reviewed update plan may run")]
        consent: bool,
        #[arg(
            long,
            help = "Idempotency key; reuse it to retry the same action safely"
        )]
        idempotency_key: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConversationCommand {
    #[command(about = "List conversations with their run counts")]
    List,
    #[command(about = "Show a conversation's runs and whether a turn is active")]
    Status {
        #[arg(long, help = "Conversation to read")]
        conversation_id: String,
    },
    #[command(about = "Show a conversation's runs, turns, and artifacts")]
    Timeline {
        #[arg(long, help = "Conversation to read")]
        conversation_id: String,
    },
    #[command(about = "Show one page of a run's activity facts")]
    Activity {
        #[arg(long, help = "Conversation to read")]
        conversation_id: String,
        #[arg(long, help = "Run to read")]
        run_id: String,
        #[arg(long, default_value_t = 0, help = "Read only facts after this cursor")]
        after_cursor: u64,
    },
    #[command(about = "Show one page of the prompts you sent in a conversation")]
    Content {
        #[arg(long, help = "Conversation to read")]
        conversation_id: String,
        #[arg(long, help = "Read only prompts before this cursor")]
        before: Option<gent_types::ConversationContentCursor>,
        #[arg(long, default_value_t = 50, help = "Maximum prompts to return")]
        limit: u16,
    },
}
