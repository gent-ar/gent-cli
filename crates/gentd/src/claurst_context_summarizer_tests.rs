use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
};

use gent_drivers::conversation_context_summary::{summary_chunks, summary_request};
use gent_types::{ContextCompactionFailure, ContextSourceItem, ContextSourceRole};
use serde_json::json;

use super::{
    ContextSummarizer, LlamaContextSummarizer, LlamaSummaryEndpoint, completion_body,
    completion_summary,
};

fn request() -> gent_drivers::conversation_context_summary::SummaryRequest {
    let chunks = summary_chunks(
        &[ContextSourceItem {
            role: ContextSourceRole::User,
            text: "Remember LARK-7".into(),
        }],
        4_096,
    );
    summary_request(None, &chunks[0])
}

#[test]
fn the_summary_request_exposes_no_tools_and_disables_gent_instructions_and_thinking() {
    let body = completion_body("qwen3-1-7b-q4-k-m", &request());
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
    assert_eq!(
        body["chat_template_kwargs"],
        json!({"gent_instructions": "", "enable_thinking": false})
    );
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(body["stream"], false);
    assert_eq!(body["max_tokens"], 1_024);
}

#[test]
fn only_a_stopped_non_empty_completion_is_a_summary() {
    let completion = |finish: &str, content: &str| json!({"choices": [{"finish_reason": finish, "message": {"content": content}}], "usage": {"completion_tokens": 42}});
    let summary = completion_summary(&completion(
        "stop",
        "<think></think>The user planted LARK-7.",
    ))
    .unwrap();
    assert_eq!(
        (summary.text.as_str(), summary.tokens),
        ("The user planted LARK-7.", 42)
    );
    assert_eq!(
        completion_summary(&completion("length", "The user planted")),
        Err(ContextCompactionFailure::OutputLimit)
    );
    assert_eq!(
        completion_summary(&completion("stop", "   ")),
        Err(ContextCompactionFailure::EmptySummary)
    );
    assert_eq!(
        completion_summary(&json!({"error": "loading model"})),
        Err(ContextCompactionFailure::RuntimeUnavailable)
    );
}

#[test]
fn the_window_reserves_the_summary_output_from_the_model_context() {
    let summarizer = LlamaContextSummarizer::new(LlamaSummaryEndpoint {
        server_url: "http://127.0.0.1:1".into(),
        model: "model".into(),
        context_tokens: 32_768,
        history_input_bytes: 81_888,
    });
    let window = summarizer.window();
    assert_eq!(window.history_input_bytes, 81_888);
    assert_eq!(window.summary_input_bytes, (32_768 - 1_024 - 512) * 3);
}

#[tokio::test]
async fn the_summarizer_posts_one_completion_to_the_running_llama_server() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, received) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4_096];
        loop {
            let count = stream.read(&mut chunk).unwrap();
            buffer.extend_from_slice(&chunk[..count]);
            let text = String::from_utf8_lossy(&buffer).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or_default();
                if body.len() >= length {
                    sender.send((head.to_owned(), body.to_owned())).unwrap();
                    break;
                }
            }
        }
        let reply = json!({"choices": [{"finish_reason": "stop", "message": {"content": "The user planted LARK-7."}}], "usage": {"completion_tokens": 9}}).to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
    });
    let summarizer = LlamaContextSummarizer::new(LlamaSummaryEndpoint {
        server_url: format!("http://{address}"),
        model: "qwen3-1-7b-q4-k-m".into(),
        context_tokens: 32_768,
        history_input_bytes: 81_888,
    });
    let summary = summarizer.summarize(request()).await.unwrap();
    assert_eq!(summary.text, "The user planted LARK-7.");
    let (head, body) = received.recv().unwrap();
    assert!(head.starts_with("POST /v1/chat/completions HTTP/1.1"));
    let body = serde_json::from_str::<serde_json::Value>(&body).unwrap();
    assert!(
        body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("Remember LARK-7")
    );
    assert!(body.get("tools").is_none());
    let unavailable = LlamaContextSummarizer::new(LlamaSummaryEndpoint {
        server_url: format!("http://{address}"),
        model: "model".into(),
        context_tokens: 32_768,
        history_input_bytes: 81_888,
    });
    assert_eq!(
        unavailable.summarize(request()).await,
        Err(ContextCompactionFailure::RuntimeUnavailable)
    );
}
