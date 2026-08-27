use gent_runtime::RuntimeError;

pub(super) fn missing_binding() -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
        "Codex runner has no durable prompt binding".into(),
    ))
}
