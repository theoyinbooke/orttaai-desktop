//! End-to-end tests of the HTTP clients against a tiny canned HTTP server on
//! loopback (no Ollama or LM Studio needed).

use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

type Route = (&'static str, u16, &'static str, String);

/// Serve each `(path, status, content-type, body)` route for any number of
/// requests; returns the base URL and a channel of received `(path, body)`.
fn serve(routes: Vec<Route>) -> (String, mpsc::Receiver<(String, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let path = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .to_string();
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                    break;
                }
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            let _ = tx.send((path.clone(), String::from_utf8_lossy(&body).into_owned()));
            let (status, ctype, payload) = routes
                .iter()
                .find(|(p, ..)| path.starts_with(p))
                .map(|(_, s, c, b)| (*s, *c, b.clone()))
                .unwrap_or((404, "application/json", r#"{"error":"not found"}"#.into()));
            let _ = write!(
                stream,
                "HTTP/1.1 {status} OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.flush();
        }
    });
    (base, rx)
}

#[test]
fn ollama_client_end_to_end() {
    let (base, rx) = serve(vec![
        (
            "/api/tags",
            200,
            "application/json",
            r#"{"models":[{"name":"gemma4:e2b","size":10},{"name":"x:cloud"}]}"#.into(),
        ),
        (
            "/api/chat",
            200,
            "application/x-ndjson",
            "{\"message\":{\"thinking\":\"hm.\"}}\n{\"message\":{\"content\":\"Hi\"}}\n{\"done\":true}\n"
                .into(),
        ),
        (
            "/api/generate",
            200,
            "application/json",
            r#"{"response":" Cleaned. ","done":true}"#.into(),
        ),
        (
            "/api/embed",
            200,
            "application/json",
            r#"{"embeddings":[[0.1,0.2]]}"#.into(),
        ),
        (
            "/api/show",
            200,
            "application/json",
            r#"{"capabilities":["completion","thinking"]}"#.into(),
        ),
        (
            "/api/pull",
            200,
            "application/x-ndjson",
            "{\"status\":\"pulling\",\"total\":4,\"completed\":2}\n{\"status\":\"success\"}\n".into(),
        ),
    ]);
    let c = ollama::OllamaClient::new(&format!("{base}/"));
    let models = c.list_models().unwrap();
    assert_eq!(models.len(), 1);
    assert!(c.health().reachable);

    let mut deltas = 0;
    let mut cb = |_d: ChatDelta| deltas += 1;
    let opts = ChatOptions {
        think: Some(true),
        json_schema: Some(serde_json::json!({"type":"object"})),
        num_ctx: Some(8_192),
        ..ChatOptions::model("gemma4:e2b")
    };
    let reply = c
        .chat(&[ChatMessage::user("hello")], &opts, Some(&mut cb))
        .unwrap();
    assert_eq!(reply.content, "Hi");
    assert_eq!(reply.reasoning.as_deref(), Some("hm."));
    assert_eq!(deltas, 2);

    let text = c
        .generate(
            &GenerateRequest::raw("<|im_start|>x", &["<|im_end|>"]),
            &ChatOptions::model("gemma4:e2b"),
        )
        .unwrap();
    assert_eq!(text, "Cleaned.");
    assert_eq!(c.embed("all-minilm", &["a".into()]).unwrap().len(), 1);
    assert_eq!(
        c.model_capabilities("gemma4:e2b").unwrap(),
        vec!["completion", "thinking"]
    );
    let mut progress = Vec::new();
    c.pull_model("all-minilm", &mut |p| progress.push(p))
        .unwrap();
    assert_eq!(progress.len(), 2);

    // Requests carried the Mac payload shape.
    let seen: Vec<(String, String)> = rx.try_iter().collect();
    let chat_body = &seen.iter().find(|(p, _)| p == "/api/chat").unwrap().1;
    let v: serde_json::Value = serde_json::from_str(chat_body).unwrap();
    assert_eq!(v["think"], true);
    assert_eq!(v["stream"], true);
    assert_eq!(v["options"]["num_ctx"], 8_192);
    assert_eq!(v["format"]["type"], "object");
    let gen_body = &seen.iter().find(|(p, _)| p == "/api/generate").unwrap().1;
    let g: serde_json::Value = serde_json::from_str(gen_body).unwrap();
    assert_eq!(g["raw"], true);
    assert_eq!(g["options"]["stop"][0], "<|im_end|>");

    // Policy: cloud models never leave the app.
    let err = c
        .chat(
            &[ChatMessage::user("x")],
            &ChatOptions::model("gpt-oss:120b-cloud"),
            None,
        )
        .unwrap_err();
    assert!(err.to_string().contains("own hardware"));
}

#[test]
fn ollama_http_errors_are_classified() {
    let (base, _rx) = serve(vec![(
        "/api/generate",
        404,
        "application/json",
        r#"{"error":"model \"nope\" not found, try pulling it first"}"#.into(),
    )]);
    let c = ollama::OllamaClient::new(&base);
    let err = c
        .generate(&GenerateRequest::new("hi"), &ChatOptions::model("nope"))
        .unwrap_err();
    assert_eq!(classify_error(&err), LlmFailure::ModelMissing);
    assert!(err.to_string().contains("not found"));

    let remote = ollama::OllamaClient::new("https://api.example.com");
    let err = remote.list_models().unwrap_err();
    assert!(err.to_string().contains("own hardware"));
}

#[test]
fn lmstudio_client_end_to_end() {
    let (base, rx) = serve(vec![
        (
            "/api/v1/models/download/status/",
            200,
            "application/json",
            r#"{"status":"completed","total_size_bytes":10,"downloaded_bytes":10}"#.into(),
        ),
        (
            "/api/v1/models/download",
            200,
            "application/json",
            r#"{"job_id":"j1","status":"downloading","total_size_bytes":10,"downloaded_bytes":1}"#
                .into(),
        ),
        (
            "/api/v1/models",
            200,
            "application/json",
            r#"{"models":[{"key":"qwen3.5-0.8b","type":"llm","size_bytes":9},{"key":"cloud-x","type":"llm","size_bytes":0}]}"#
                .into(),
        ),
        (
            "/v1/chat/completions",
            200,
            "text/event-stream",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"r\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"Yo\"}}]}\n\ndata: [DONE]\n\n"
                .into(),
        ),
        (
            "/v1/completions",
            200,
            "application/json",
            r#"{"choices":[{"text":"Polished."}]}"#.into(),
        ),
    ]);
    let c = lmstudio::LmStudioClient::new(&format!("{base}/v1"));
    assert_eq!(c.list_models().unwrap().len(), 1);
    let opts = ChatOptions {
        json_schema: Some(serde_json::json!({"type":"object"})),
        max_tokens: Some(100),
        ..ChatOptions::model("qwen3.5-0.8b")
    };
    let r = c.chat(&[ChatMessage::user("hi")], &opts, None).unwrap();
    assert_eq!(r.content, "Yo");
    assert_eq!(r.reasoning.as_deref(), Some("r"));
    assert_eq!(
        c.generate(
            &GenerateRequest::raw("p", &["<|im_end|>"]),
            &ChatOptions::model("qwen3.5-0.8b")
        )
        .unwrap(),
        "Polished."
    );
    // Unknown ids fail loudly instead of LM Studio swapping models.
    let err = c
        .chat(
            &[ChatMessage::user("hi")],
            &ChatOptions::model("missing"),
            None,
        )
        .unwrap_err();
    assert_eq!(classify_error(&err), LlmFailure::ModelMissing);
    assert_eq!(
        c.model_capabilities("qwen3.5-0.8b").unwrap(),
        vec!["completion"]
    );
    let mut states = Vec::new();
    c.pull_model(
        "https://huggingface.co/superwhisper/s1-mini-GGUF",
        &mut |p| states.push(p.status),
    )
    .unwrap();
    assert_eq!(states.last().map(String::as_str), Some("Downloaded"));

    let seen: Vec<(String, String)> = rx.try_iter().collect();
    let body = &seen
        .iter()
        .find(|(p, _)| p == "/v1/chat/completions")
        .unwrap()
        .1;
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(v["response_format"]["type"], "json_schema");
    assert_eq!(v["ttl"], 300);
    assert_eq!(v["max_tokens"], 100);
}
