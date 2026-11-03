use super::*;

#[test]
fn fails_closed_for_server_to_client_requests_without_wedging_the_turn() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    let dynamic_tool = driver
        .receive(br#"{"jsonrpc":"2.0","id":"private-request","method":"item/tool/call","params":{"tool":"private"}}"#)
        .unwrap();
    assert_eq!(
        frames(&dynamic_tool),
        vec![json!({
            "id":"private-request",
            "result":{"success":false,"contentItems":[{
                "type":"inputText",
                "text":"Gent has no registered executor for this experimental Codex dynamic tool."
            }]}
        })]
    );
    let auth = driver
        .receive(br#"{"jsonrpc":"2.0","id":9,"method":"account/chatgptAuthTokens/refresh"}"#)
        .unwrap();
    assert_eq!(frames(&auth)[0]["error"]["code"], json!(-32603));
    let malformed = driver
        .receive(br#"{"jsonrpc":"2.0","id":null,"method":"attestation/generate"}"#)
        .unwrap();
    assert!(matches!(
        malformed.as_slice(),
        [CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification }))]
            if classification == "malformedCodexClientRequest"
    ));
    let next = driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    assert_eq!(frames(&next)[1]["method"], json!("thread/start"));
}

#[test]
fn answers_every_unrecognized_codex_server_request_with_an_error_so_codex_never_waits() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    let unknown = driver
        .receive(br#"{"jsonrpc":"2.0","id":41,"method":"item/future/requestDecision","params":{"secret":"private"}}"#)
        .unwrap();
    assert_eq!(
        frames(&unknown),
        vec![
            json!({"id":41,"error":{"code":-32601,"message":"Gent does not handle this Codex server request."}})
        ]
    );
    assert!(unknown.iter().any(|effect| matches!(
        effect,
        CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification }))
            if classification == "unsupportedCodexServerRequest"
    )));
    assert!(!format!("{unknown:?}").contains("private"));
    let malformed_approval = driver
        .receive(br#"{"jsonrpc":"2.0","id":"approval-1","method":"item/commandExecution/requestApproval"}"#)
        .unwrap();
    assert_eq!(frames(&malformed_approval)[0]["id"], json!("approval-1"));
    assert_eq!(
        frames(&malformed_approval)[0]["error"]["code"],
        json!(-32601)
    );
    let notification = driver
        .receive(br#"{"jsonrpc":"2.0","method":"item/future/updated","params":{}}"#)
        .unwrap();
    assert!(frames(&notification).is_empty());
    let next = driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    assert_eq!(frames(&next)[1]["method"], json!("thread/start"));
}
