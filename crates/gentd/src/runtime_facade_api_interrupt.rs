impl RuntimeFacade {
    fn interrupt_intent(
        &self,
        host_epoch: gent_types::HostEpoch,
        frame: gent_protocol::AgentChatIntentFrame,
    ) -> Result<Option<gent_protocol::AgentChatIntentFrame>, String> {
        let gent_protocol::AgentChatIntentFrame::Interrupt { request_id, receipt_id, conversation_id, run_id } = frame else {
            return Ok(None);
        };
        let ingress = self.ordinary_prompt_ingress.as_ref().ok_or_else(|| "agent-chat provider lifecycle is not configured".to_owned())?;
        let selection = self.agent_chat_reads.as_ref().ok_or_else(|| "agent-chat reads are unavailable".to_owned())?
            .run_selection(&conversation_id.0, &run_id.0).map_err(|error| error.to_string())?;
        let receipt = self.coordinator.submit(&gent_types::Command {
            receipt_id: receipt_id.clone(),
            idempotency_key: format!("agent-chat-interrupt:{}", request_id.0),
            host_epoch,
            kind: "agentChatInterrupt".into(),
            payload: serde_json::json!({ "conversationId": conversation_id, "runId": run_id }),
        }).map_err(|error| error.to_string())?;
        ingress.interrupt_run(selection.provider, &run_id.0)?;
        Ok(Some(gent_protocol::AgentChatIntentFrame::Interrupted { request_id, receipt, conversation_id, run_id }))
    }
}
