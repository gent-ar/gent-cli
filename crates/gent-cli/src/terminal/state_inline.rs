use gent_types::PermissionMode;

use super::{UiCommand, UiEffect, UiState, permissions};

pub(crate) const PERMISSION_CHOICES: [(&str, &str); 4] = [
    ("/approve", "Approve once"),
    ("/approve-tool", "Always allow this tool"),
    ("/approve-category", "Always allow this category"),
    ("/deny", "Deny"),
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct InlineState {
    pub(crate) permission_cursor: usize,
    pub(crate) bypass_confirm: bool,
    pub(crate) quit_armed: bool,
    pub(crate) palette_cursor: usize,
    pub(crate) help_scroll: u16,
}

pub(crate) const fn next_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::AskEveryTime => PermissionMode::AutoAcceptEdits,
        PermissionMode::AutoAcceptEdits => PermissionMode::Autonomous,
        PermissionMode::Autonomous => PermissionMode::Bypass,
        PermissionMode::Bypass => PermissionMode::AskEveryTime,
    }
}

impl UiState {
    #[must_use]
    pub(crate) const fn inline(&self) -> &InlineState {
        &self.inline
    }

    #[must_use]
    pub(crate) fn mode_applies_next_turn(&self) -> bool {
        self.selected_status().is_some_and(|status| {
            status.runs.iter().any(|run| {
                run.active_turn_id.is_some()
                    || run
                        .live_status
                        .as_ref()
                        .is_some_and(|live| live.status.needs_attention())
            })
        })
    }

    #[must_use]
    pub(crate) fn palette_open(&self) -> bool {
        self.command_catalog().is_some_and(|catalog| {
            !super::super::commands::palette(catalog, &self.input).is_empty()
        })
    }

    pub(super) fn inline_apply(&mut self, command: &UiCommand) -> Option<UiEffect> {
        if !matches!(command, UiCommand::Interrupt) {
            self.inline.quit_armed = false;
        }
        if self.inline.bypass_confirm {
            return Some(self.bypass_answer(command));
        }
        match command {
            UiCommand::CyclePermissionMode => Some(self.cycle_permission_mode()),
            UiCommand::Interrupt if !self.turn_active() => Some(self.arm_quit()),
            UiCommand::SelectNext | UiCommand::SelectPrevious if self.help_visible() => {
                let step = if *command == UiCommand::SelectNext {
                    1
                } else {
                    -1
                };
                self.inline.help_scroll = self.inline.help_scroll.saturating_add_signed(step);
                Some(UiEffect::Continue)
            }
            _ if self.input.is_empty() && self.selected_pending_permission().is_some() => {
                self.permission_key(command)
            }
            UiCommand::SelectNext | UiCommand::SelectPrevious if self.palette_open() => {
                let count = self.command_catalog().map_or(0, |catalog| {
                    super::super::commands::palette(catalog, &self.input).len()
                });
                let cursor = self.inline.palette_cursor;
                self.inline.palette_cursor = if *command == UiCommand::SelectNext {
                    (cursor + 1).min(count.saturating_sub(1))
                } else {
                    cursor.saturating_sub(1)
                };
                Some(UiEffect::Continue)
            }
            UiCommand::SubmitPrompt if self.palette_open() => {
                self.complete_palette();
                None
            }
            UiCommand::Insert(_) | UiCommand::DeleteInput | UiCommand::Paste(_) => {
                self.inline.palette_cursor = 0;
                None
            }
            _ => None,
        }
    }

    fn complete_palette(&mut self) {
        let typed = self.input.trim_start_matches('/').to_owned();
        let Some(catalog) = self.command_catalog() else {
            return;
        };
        let matches = super::super::commands::palette(catalog, &self.input);
        if matches.iter().any(|command| command.name == typed) {
            return;
        }
        if let Some(command) = matches.get(self.inline.palette_cursor.min(matches.len() - 1)) {
            self.input = format!("/{}", command.name);
        }
    }

    fn permission_key(&mut self, command: &UiCommand) -> Option<UiEffect> {
        let choice = match command {
            UiCommand::Insert(digit @ '1'..='4') => {
                usize::from(u8::try_from(*digit).unwrap_or(b'1') - b'1')
            }
            UiCommand::SubmitPrompt => self.inline.permission_cursor,
            UiCommand::SelectNext => {
                self.inline.permission_cursor = (self.inline.permission_cursor + 1).min(3);
                return Some(UiEffect::Continue);
            }
            UiCommand::SelectPrevious => {
                self.inline.permission_cursor = self.inline.permission_cursor.saturating_sub(1);
                return Some(UiEffect::Continue);
            }
            _ => return None,
        };
        self.inline.permission_cursor = 0;
        permissions::command(self, PERMISSION_CHOICES[choice].0, "")
    }

    fn cycle_permission_mode(&mut self) -> UiEffect {
        let next = next_mode(self.permission_mode());
        if next == PermissionMode::Bypass {
            self.inline.bypass_confirm = true;
            return UiEffect::Continue;
        }
        permissions::mode_request(self, next, false)
    }

    fn bypass_answer(&mut self, command: &UiCommand) -> UiEffect {
        self.inline.bypass_confirm = false;
        match command {
            UiCommand::SubmitPrompt => {
                permissions::mode_request(self, PermissionMode::Bypass, true)
            }
            UiCommand::CyclePermissionMode => {
                permissions::mode_request(self, PermissionMode::AskEveryTime, false)
            }
            _ => UiEffect::Continue,
        }
    }

    fn arm_quit(&mut self) -> UiEffect {
        if self.inline.quit_armed {
            return UiEffect::Quit;
        }
        self.inline.quit_armed = true;
        self.notice = Some("Press Ctrl+C again to quit.".into());
        UiEffect::Continue
    }
}

impl UiState {
    #[must_use]
    pub(crate) fn with_selected_conversation(mut self, conversation_id: Option<&str>) -> Self {
        if let Some(conversation_id) = conversation_id {
            self.select_conversation(conversation_id);
        }
        self
    }

    pub(crate) fn merge_conversations(
        &mut self,
        mut conversations: Vec<gent_types::ConversationListItem>,
        metadata: std::collections::BTreeMap<String, super::super::ConversationMetadata>,
    ) {
        let selected = self.selected().map(|item| item.conversation_id.clone());
        for (conversation_id, value) in metadata {
            let entry = self.metadata.entry(conversation_id).or_default();
            entry.title = value.title.or(entry.title.take());
            entry.workspace_id = value.workspace_id.or(entry.workspace_id.take());
            entry.workspace_path = value.workspace_path.or(entry.workspace_path.take());
            entry.updated_at_unix_ms = value.updated_at_unix_ms;
            entry.running = value.running;
        }
        conversations.sort_by_key(|item| {
            std::cmp::Reverse(
                self.metadata
                    .get(&item.conversation_id)
                    .map_or(0, |value| value.updated_at_unix_ms),
            )
        });
        self.conversations = conversations;
        self.selected = selected.and_then(|id| {
            self.conversations
                .iter()
                .position(|item| item.conversation_id == id)
        });
        if self.commands.picker.is_some() {
            self.commands.picker = Some(self.selected.unwrap_or(0));
        }
    }
}
