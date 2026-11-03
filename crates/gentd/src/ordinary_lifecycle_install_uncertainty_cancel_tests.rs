use super::*;
use gent_ports::AgentChatPromptDispatchLedger;

#[test]
fn successor_cancels_exact_original_turn_after_uncertain_or_interrupted_install() {
    for installer_returned in [true, false] {
        let (directory, ledger, old) = setup();
        let prompt = held(&ledger, &old, "installer-uncertainty");
        let (binding, command, receipt) = reserve_install(&ledger, &prompt);
        if installer_returned {
            ledger
                .settle_unprovable_provider_prompt_provision(
                    &command,
                    &receipt,
                    &Event {
                        cursor: 0,
                        event_id: "uncertain-install".into(),
                        receipt_id: receipt.receipt_id.clone(),
                        host_epoch: command.host_epoch,
                        kind: "privatePromptProvisionUnprovable".into(),
                        payload: command.payload.clone(),
                    },
                    &binding,
                )
                .unwrap();
        }
        ledger.close_ingress(HostEpoch(1)).unwrap();
        drop(old);
        drop(ledger);
        let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
        let epoch = ledger.fence_and_open(HostEpoch(1)).unwrap().epoch;
        ledger.recover_agent_chat_prompt_dispatches(epoch).unwrap();
        let fresh = StandaloneReadiness::new_with_public_readiness(
            ledger.clone(),
            epoch,
            None,
            Arc::new(NotInstalled),
        );
        assert!(
            !fresh
                .cancel_held_turn(&prompt.run, "stale-other-turn")
                .unwrap()
        );
        assert_still_held(&ledger, &prompt);
        let held = ledger
            .read_awaiting_agent_chat_prompts(Some(&prompt.run), "", 128)
            .unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].receipt.receipt_id.0, prompt.receipt);
        assert_eq!(held[0].message.turn_id, prompt.turn_id);
        assert!(
            fresh
                .cancel_held_turn(&prompt.run, &prompt.turn_id)
                .unwrap()
        );
        assert!(
            !fresh
                .cancel_held_turn(&prompt.run, &prompt.turn_id)
                .unwrap()
        );
        assert_cancelled(&ledger, &prompt);
        assert!(
            ledger
                .claim_agent_chat_prompt_dispatch("successor", epoch, AgentChatProvider::Codex)
                .unwrap()
                .is_none()
        );
    }
}
