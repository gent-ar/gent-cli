//! Read-only terminal shell. Rendering and input remain independent of local IPC.

mod input;
mod render;
mod state;
mod terminal_loop;

pub(crate) use state::UiState;
pub(crate) use terminal_loop::run;
