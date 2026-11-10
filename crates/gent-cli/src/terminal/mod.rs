mod commands;
mod effects;
mod inline_items;
mod inline_live;
mod inline_loop;
mod inline_popups;
mod inline_tools;
mod inline_view;
mod input;
mod render;
mod render_activity;
mod render_activity_panel;
mod render_commands;
mod render_composer;
mod render_header;
mod render_help;
mod render_permission;
mod render_picker;
mod render_plan;
mod render_processes;
mod render_selection_picker;
mod render_sidebar;
mod render_subagents;
mod render_timeline;
mod render_tools;
mod selection;
mod state;
mod state_switch;
mod state_view;
mod terminal_loop;
mod view;

pub(crate) use inline_loop::run_inline;
pub(crate) use state::{UiRequest, UiRequestResult, UiState};
pub(crate) use terminal_loop::{TerminalPorts, WorkspaceGitResult, require_interactive, run};
pub(crate) use view::{ConversationMetadata, ConversationView};
#[cfg(test)]
mod inline_tests;
#[cfg(test)]
mod inline_tests_live;
#[cfg(test)]
mod inline_tests_scrollback;
#[cfg(test)]
mod inline_tests_sign_in;
