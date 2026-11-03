use gent_ports::{IngressMode, LedgerError};
use gent_types::HostEpoch;

use super::{
    SqliteLedger,
    epoch::require_epoch,
    queries::{host_ingress, storage_error},
};

impl SqliteLedger {
    pub fn quiesce_idle_owner(&self, epoch: HostEpoch) -> Result<(), LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let ingress = host_ingress(&transaction)?;
        require_epoch(epoch, ingress.epoch)?;
        if ingress.mode != IngressMode::Open {
            return Err(LedgerError::IngressClosed { epoch });
        }
        let busy: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_chat_prompt_dispatches WHERE state IN ('provisioning', 'claimed', 'launching', 'started')) OR EXISTS(SELECT 1 FROM pending_provider_permissions)",
            [], |row| row.get(0),
        ).map_err(storage_error)?;
        if busy {
            return Err(LedgerError::Invariant(
                "runtime owner has active prompt work".into(),
            ));
        }
        transaction
            .execute(
                "UPDATE host_state SET ingress = 'closed' WHERE singleton = 1",
                [],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gent_ports::Ledger;

    fn ledger(state: &str) -> SqliteLedger {
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger.lock().unwrap().execute_batch(
            "INSERT INTO conversations(conversation_id) VALUES ('conversation');
             INSERT INTO runs(run_id, provider, conversation_id) VALUES ('run','codex','conversation');
             INSERT INTO turns(turn_id,conversation_id,run_id,sequence,phase) VALUES ('turn','conversation','run',1,'queued');
             INSERT INTO conversation_messages(message_id,turn_id,conversation_id,run_id,text,text_digest_sha256,byte_len) VALUES ('message','turn','conversation','run','held','digest',4);"
        ).unwrap();
        ledger.lock().unwrap().execute(
            "INSERT INTO agent_chat_prompt_dispatches(message_id,state,created_rowid) VALUES ('message',?1,1)", [state],
        ).unwrap();
        ledger
    }

    #[test]
    fn active_dispatch_states_preserve_open_ingress() {
        for state in ["provisioning", "claimed", "launching", "started"] {
            let ledger = ledger(state);
            assert!(ledger.quiesce_idle_owner(HostEpoch(1)).is_err());
            assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Open);
        }
    }

    #[test]
    fn pending_prompts_remain_unchanged_when_idle_owner_closes() {
        let ledger = ledger("pending");
        ledger.quiesce_idle_owner(HostEpoch(1)).unwrap();
        let state: String = ledger
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM agent_chat_prompt_dispatches WHERE message_id='message'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "pending");
        assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Closed);
    }

    #[test]
    fn stale_epoch_cannot_close_current_owner() {
        let ledger = ledger("pending");
        assert!(ledger.quiesce_idle_owner(HostEpoch(2)).is_err());
        assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Open);
    }

    #[test]
    fn unresolved_permission_blocks_repair_even_if_dispatch_is_settled() {
        let ledger = ledger("settled");
        ledger.lock().unwrap().execute(
            "INSERT INTO pending_provider_permissions(decision_id,conversation_id,run_id,binding_json,request_json) VALUES ('decision','conversation','run','{}','{}')", [],
        ).unwrap();
        assert!(ledger.quiesce_idle_owner(HostEpoch(1)).is_err());
        assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Open);
    }
}
