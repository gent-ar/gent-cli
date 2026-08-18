//! Read-only terminal shell. Rendering and input remain independent of local IPC.

#[allow(dead_code)] // Stream capability is not advertised by the observer daemon yet.
mod chat_projection;
#[allow(dead_code)] // The observer daemon deliberately does not compose this stream yet.
mod controller_stream;
mod input;
mod render;
mod selection;
mod state;
mod state_switch;
mod terminal_loop;

pub(crate) use state::{UiRequest, UiRequestResult, UiState};
pub(crate) use terminal_loop::{require_interactive, run};
