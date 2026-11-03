use super::*;
use crate::codex_session::CodexTurnOptions;
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};

fn config(model: &str, resume: bool) -> CodexSessionConfig {
    CodexSessionConfig {
        working_directory: Some("/owned".into()),
        resume_thread_id: resume.then(|| "native-thread".into()),
        turn_options: CodexTurnOptions::from_selection(
            &AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: model.into(),
                effort: AgentChatEffort::High,
                mode: AgentChatMode::Plan,
            },
            Some("/owned"),
        )
        .unwrap(),
        mcp_servers: None,
    }
}

fn frames(ingress: CodexSessionIngress) -> Vec<Value> {
    let CodexSessionIngress::Send(frames) = ingress else {
        panic!("expected frames")
    };
    frames
        .iter()
        .map(|bytes| serde_json::from_slice(bytes).unwrap())
        .collect()
}

fn default_session(resume: bool) -> CodexAppServerSession {
    let (mut session, _) = CodexAppServerSession::start(config("default", resume)).unwrap();
    let opening = frames(session.receive(&json!({"id":1,"result":{}})).unwrap());
    assert_eq!(opening[0]["method"], "initialized");
    assert_eq!(opening[1]["method"], "model/list");
    assert_eq!(opening[1]["params"]["includeHidden"], false);
    assert!(!session.is_ready());
    assert_eq!(
        session.start_turn("not yet"),
        Err(CodexSessionError::ThreadNotReady)
    );
    session
}

#[test]
fn newly_installed_default_is_provider_published_not_a_literal_or_first_model() {
    for resume in [false, true] {
        let mut session = default_session(resume);
        let page = frames(
            session
                .receive(&json!({"id":2,"result":{"data":[
            {"model":"first-but-not-default","isDefault":false}
        ],"nextCursor":"page2"}}))
                .unwrap(),
        );
        assert_eq!(page[0]["method"], "model/list");
        assert_eq!(page[0]["params"]["cursor"], "page2");
        let thread = frames(
            session
                .receive(&json!({"id":3,"result":{"data":[
            {"id":"opaque-entry-id","model":"provider-default-after-install","isDefault":true}
        ],"nextCursor":null}}))
                .unwrap(),
        );
        assert_eq!(
            thread[0]["method"],
            if resume {
                "thread/resume"
            } else {
                "thread/start"
            }
        );
        assert_eq!(
            thread[0]["params"]["model"],
            "provider-default-after-install"
        );
        if resume {
            assert_eq!(thread[0]["params"]["threadId"], "native-thread");
        }
        session
            .receive(&json!({"id":4,"result":{"thread":{"id":"native-thread"}}}))
            .unwrap();
        let turn: Value =
            serde_json::from_slice(&session.start_turn("one user prompt").unwrap()).unwrap();
        assert_eq!(turn["params"]["model"], "provider-default-after-install");
        assert_eq!(turn["params"]["effort"], "high");
        assert_eq!(
            turn["params"]["collaborationMode"]["settings"]["model"],
            "provider-default-after-install"
        );
    }
}

#[test]
fn explicit_model_never_queries_or_substitutes_provider_default() {
    let (mut session, _) =
        CodexAppServerSession::start(config("explicit-future-model", false)).unwrap();
    let opening = frames(session.receive(&json!({"id":1,"result":{}})).unwrap());
    assert_eq!(opening[1]["method"], "thread/start");
    session
        .receive(&json!({"id":2,"result":{"thread":{"id":"native-thread"}}}))
        .unwrap();
    let turn: Value = serde_json::from_slice(&session.start_turn("one prompt").unwrap()).unwrap();
    assert_eq!(turn["params"]["model"], "explicit-future-model");
}

#[test]
fn no_available_default_or_ambiguous_metadata_never_launches_a_thread() {
    for data in [
        json!([]),
        json!([{"model":"first","isDefault":false}]),
        json!([{"model":"hidden","isDefault":true,"hidden":true}]),
        json!([{"model":"a","isDefault":true},{"model":"b","isDefault":true}]),
        json!([{"model":"default","isDefault":true}]),
        json!([{"isDefault":true}]),
    ] {
        let mut session = default_session(false);
        assert_eq!(
            session.receive(&json!({"id":2,"result":{"data":data}})),
            Err(CodexSessionError::ProviderDefaultUnavailable)
        );
        assert!(!session.is_ready());
        assert_eq!(
            session.start_turn("never"),
            Err(CodexSessionError::SessionFailed)
        );
    }
}

#[test]
fn truncated_repeating_or_malformed_pagination_fails_without_first_model_fallback() {
    for repeating in [false, true] {
        let mut session = default_session(false);
        for page in 1..=8_u64 {
            let cursor = if repeating {
                "loop".into()
            } else {
                format!("next-{page}")
            };
            let response = session.receive(&json!({"id":page+1,"result":{"data":[
                {"model":"advertised","isDefault":true}
            ],"nextCursor":cursor}}));
            if repeating && page == 2 || page == 8 {
                assert_eq!(response, Err(CodexSessionError::ProviderDefaultUnavailable));
                break;
            }
            assert_eq!(frames(response.unwrap())[0]["method"], "model/list");
        }
        assert!(!session.is_ready());
    }
    let mut session = default_session(false);
    assert_eq!(
        session.receive(&json!({"id":2,"result":{"data":[],"nextCursor":7}})),
        Err(CodexSessionError::MalformedResponse)
    );
}

#[test]
fn default_summary_uses_same_resolution_and_rejected_or_stale_responses_do_not_release() {
    let mut cfg = config("default", false);
    cfg.turn_options = CodexTurnOptions::summary("default").unwrap();
    let (mut session, _) = CodexAppServerSession::start(cfg).unwrap();
    assert_eq!(
        frames(session.receive(&json!({"id":1,"result":{}})).unwrap())[1]["method"],
        "model/list"
    );
    assert_eq!(
        session.receive(&json!({"id":99,"result":{"data":[]}})),
        Err(CodexSessionError::UncorrelatedResponse)
    );
    assert_eq!(
        session.receive(&json!({"id":2,"error":{"code":-1,"message":"no list"}})),
        Err(CodexSessionError::RequestRejected)
    );
    assert_eq!(
        session.start_turn("never"),
        Err(CodexSessionError::SessionFailed)
    );
}
