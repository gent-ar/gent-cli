//! Read-only terminal shell. Rendering and input remain independent of local IPC.

#[allow(dead_code)] // Stream capability is not advertised by the observer daemon yet.
mod chat_projection;
mod input;
mod render;
mod state;
mod terminal_loop;

pub(crate) use state::{UiRequest, UiRequestResult, UiState};
pub(crate) use terminal_loop::{require_interactive, run};
