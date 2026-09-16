use std::{
    cell::{Cell, RefCell},
    convert::Infallible,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gent_types::{
    AgentChatPromptDelivery, AgentChatProvider, ConversationActivityFact, DurableTurnPhase,
    PromptHoldReason, TurnTerminal, TurnTerminalCause,
};

use super::{
    turn_follow::{self, FollowItem},
    turn_view::{Output, TurnView},
};
use crate::{
    conversation_activity, local_models_cli, permissions_cli,
    prompt_hold::{self, PromptHold},
};

const IDLE_POLL: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TurnTarget {
    pub(crate) conversation: String,
    pub(crate) run: String,
    pub(crate) turn: String,
}

pub(crate) async fn follow(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    target: &TurnTarget,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let view = RefCell::new(
        TurnView::new(
            &command_prefix(data_dir.as_deref()),
            &target.conversation,
            &target.run,
        )
        .with_live_progress(io::stderr().is_terminal()),
    );
    let last_event = Cell::new(Instant::now());
    let mut sink = |item: FollowItem<'_>| {
        if let FollowItem::Event(event) = &item {
            view.borrow_mut().event(event);
            last_event.set(Instant::now());
        }
        if json {
            view.borrow_mut().take();
            turn_follow::print_json(item)
        } else {
            flush(&view)
        }
    };
    let followed = turn_follow::follow_accepted_if_supported(
        data_dir.clone(),
        no_autostart,
        target.conversation.clone(),
        target.run.clone(),
        target.turn.clone(),
        &mut sink,
    );
    let terminal = if json {
        followed.await?
    } else {
        tokio::select! {
            terminal = followed => terminal?,
            never = watch(data_dir.clone(), no_autostart, target, &view, &last_event) => match never {},
        }
    };
    let Some(terminal) = terminal else {
        return Ok(());
    };
    let cause = terminal_cause(data_dir, no_autostart, &terminal).await;
    let outcome = view.borrow_mut().finish(terminal.phase, cause);
    if json {
        view.borrow_mut().take();
    } else {
        flush(&view)?;
    }
    outcome.map_err(Into::into)
}

pub(crate) fn announce_delivery(
    data_dir: Option<&Path>,
    conversation_id: &str,
    delivery: AgentChatPromptDelivery,
) {
    if delivery == AgentChatPromptDelivery::Queued {
        eprintln!(
            "Queued behind the running turn. Deliver it now with: {} chat steer --conversation-id {conversation_id}",
            command_prefix(data_dir)
        );
    }
}

pub(crate) fn command_prefix(data_dir: Option<&Path>) -> String {
    data_dir.map_or_else(
        || "gent".to_owned(),
        |directory| {
            let directory = directory.display().to_string();
            if directory.chars().any(char::is_whitespace) {
                format!("gent --data-dir '{directory}'")
            } else {
                format!("gent --data-dir {directory}")
            }
        },
    )
}

fn flush(view: &RefCell<TurnView>) -> io::Result<()> {
    let outputs = view.borrow_mut().take();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    for output in outputs {
        match output {
            Output::Reply(text) => {
                stdout.write_all(text.as_bytes())?;
                stdout.flush()?;
            }
            Output::Status(text) => {
                stderr.write_all(text.as_bytes())?;
                stderr.flush()?;
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct WatchState {
    activity_cursor: u64,
    hold: Option<PromptHold>,
    reviewed_hold: Option<String>,
    selection: Option<(AgentChatProvider, String)>,
}

async fn watch(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    target: &TurnTarget,
    view: &RefCell<TurnView>,
    last_event: &Cell<Instant>,
) -> Infallible {
    let mut state = WatchState::default();
    loop {
        tokio::time::sleep(IDLE_POLL).await;
        if last_event.get().elapsed() < IDLE_POLL {
            continue;
        }
        let _ = poll(data_dir.clone(), no_autostart, target, view, &mut state).await;
        let _ = flush(view);
    }
}

async fn poll(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    target: &TurnTarget,
    view: &RefCell<TurnView>,
    state: &mut WatchState,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(Some(request)) = permissions_cli::agent_chat::pending(
        data_dir.clone(),
        no_autostart,
        target.conversation.clone(),
        target.run.clone(),
    )
    .await
        && request.binding.turn_id == target.turn
    {
        view.borrow_mut().permission(&request);
    }
    loop {
        let page = conversation_activity::request(
            data_dir.clone(),
            no_autostart,
            target.conversation.clone(),
            target.run.clone(),
            state.activity_cursor,
        )
        .await?
        .0;
        for fact in &page.facts {
            state.activity_cursor = state.activity_cursor.max(fact.scope().cursor);
            if fact.scope().turn_id == target.turn {
                state.hold = prompt_hold::hold_after(state.hold.take(), fact);
            }
        }
        match page.next_after_cursor {
            Some(next) => state.activity_cursor = state.activity_cursor.max(next),
            None => break,
        }
    }
    let Some(hold) = state.hold.clone() else {
        return Ok(());
    };
    if state.selection.is_none() {
        let detail =
            super::reads::detail(data_dir.clone(), no_autostart, target.conversation.clone())
                .await?;
        state.selection = detail
            .runs
            .into_iter()
            .find(|run| run.run_id == target.run)
            .map(|run| (run.selection.provider, run.selection.model));
    }
    let Some((provider, model)) = state.selection.clone() else {
        return Ok(());
    };
    match hold.reason {
        PromptHoldReason::ProviderInstall => {
            if state.reviewed_hold.as_ref() == Some(&hold.receipt_id) {
                return Ok(());
            }
            match prompt_hold::install_review(
                data_dir,
                no_autostart,
                &target.conversation,
                &target.run,
                &hold,
            )
            .await
            {
                Ok(Some(install)) => {
                    state.reviewed_hold = Some(hold.receipt_id.clone());
                    view.borrow_mut().install_hold(&install);
                }
                Ok(None) => {}
                Err(_) => view.borrow_mut().provider_install(provider),
            }
        }
        PromptHoldReason::ModelDownload => {
            let install = local_models_cli::status(data_dir, no_autostart, model.clone()).await?;
            view.borrow_mut().download(&model, &install);
        }
    }
    Ok(())
}

async fn terminal_cause(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    terminal: &TurnTerminal,
) -> Option<TurnTerminalCause> {
    if !matches!(
        terminal.phase,
        DurableTurnPhase::Interrupted | DurableTurnPhase::Failed
    ) {
        return None;
    }
    conversation_activity::all(
        data_dir,
        no_autostart,
        terminal.conversation_id.clone(),
        terminal.run_id.clone(),
    )
    .await
    .ok()?
    .into_iter()
    .rev()
    .find_map(|fact| match fact {
        ConversationActivityFact::Terminal { scope, cause, .. }
            if scope.turn_id == terminal.turn_id =>
        {
            cause
        }
        _ => None,
    })
}
