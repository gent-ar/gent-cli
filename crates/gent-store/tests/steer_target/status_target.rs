use super::*;
use gent_ports::RunLifecycleFactLedger;
use gent_types::AgentChatProvider;

fn steer_like_a_client(
    ledger: &SqliteLedger,
    conversation: &AgentChatConversationId,
    queued: &AgentChatPromptSaved,
    receipt: &str,
) -> Result<gent_types::Receipt, gent_ports::LedgerError> {
    let turn = ledger
        .run_active_turn(&queued.run_id.0)
        .unwrap()
        .expect("the status read shows the running turn");
    ledger
        .steer_queued_agent_chat_prompt_for_turn(
            &ReceiptId(receipt.into()),
            HostEpoch(1),
            conversation,
            &queued.message.message_id,
            &queued.run_id,
            &turn,
        )
        .map(|(receipt, _)| receipt)
}

#[test]
fn send_now_targets_the_running_turn_a_client_reads_from_status_while_a_prompt_joins_it() {
    for provider in [AgentChatProvider::Claude, AgentChatProvider::Claurst] {
        let (ledger, conversation) = populate_for(SqliteLedger::in_memory().unwrap(), provider);
        let running = active_for(&ledger, &conversation, provider);
        let [first, second] =
            ["first", "second"].map(|key| prompt(&ledger, &conversation, key, Disposition::Queue));
        let active_turn = || ledger.run_active_turn(&running.run_id.0).unwrap();
        assert_eq!(
            active_turn(),
            Some(running.message.turn_id.clone()),
            "{provider:?}"
        );
        if provider == AgentChatProvider::Claude {
            assert_eq!(claim(&ledger, &running).unwrap().message, first.message);
        }
        assert_eq!(
            active_turn(),
            Some(running.message.turn_id.clone()),
            "{provider:?}"
        );
        steer_like_a_client(&ledger, &conversation, &second, "second-steer").unwrap();
        finish(&ledger, &running);
        assert_eq!(active_turn(), None, "{provider:?}");
    }
}
