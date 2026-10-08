//! End-to-end streaming through the real HTTP stack against a loopback test server:
//! 429 + `retry-after` retry, auth failure, SSE delivery, idle timeout and cancellation.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use soyuz_ai::types::{ChatRequest, Event, Message, StopReason};
use soyuz_ai::{AiError, NetSettings, Provider, Secret, no_wake};
use soyuz_core::config::{AiProfile, AiProviderKind};

/// Serves each connection with the next canned response (`None` = hang forever).
fn server(responses: Vec<Option<String>>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    std::thread::spawn(move || {
        for (i, stream) in listener.incoming().enumerate() {
            let Ok(mut s) = stream else { break };
            h.fetch_add(1, Ordering::SeqCst);
            read_request(&mut s);
            match responses.get(i).cloned().flatten() {
                Some(r) => {
                    let _ = s.write_all(r.as_bytes());
                }
                None => std::thread::sleep(Duration::from_secs(30)),
            }
        }
    });
    (format!("http://{addr}"), hits)
}

fn read_request(s: &mut TcpStream) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; len];
    let _ = r.read_exact(&mut body);
}

fn sse_ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn provider(base: &str, net: NetSettings) -> Provider {
    let p = AiProfile {
        name: "test".into(),
        kind: AiProviderKind::Anthropic,
        base_url: base.into(),
        model: "claude-test".into(),
        ..Default::default()
    };
    Provider::with_key(&p, Some(Secret::new("sk-test")), net).unwrap()
}

const STREAM: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":1}}}\n\n\
event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n\
event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

fn req() -> ChatRequest {
    ChatRequest::new("claude-test", vec![Message::user("hello")])
}

#[test]
fn retries_429_then_streams() {
    let busy = "HTTP/1.1 429 Too Many Requests\r\nretry-after: 0\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}"
        .to_string();
    let (base, hits) = server(vec![Some(busy), Some(sse_ok(STREAM))]);
    let c = provider(&base, NetSettings::default()).stream(req(), no_wake()).collect();
    assert_eq!(c.error, None);
    assert_eq!(c.text, "hi");
    assert_eq!(c.usage.output_tokens, 3);
    assert_eq!(c.stop, Some(StopReason::EndTurn));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn auth_errors_are_not_retried() {
    let body = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
    let resp =
        format!("HTTP/1.1 401 Unauthorized\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
    let (base, hits) = server(vec![Some(resp.clone()), Some(resp)]);
    let c = provider(&base, NetSettings::default()).stream(req(), no_wake()).collect();
    assert!(matches!(c.error, Some(AiError::Auth { status: 401, .. })), "{:?}", c.error);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[test]
fn header_timeout_and_cancel() {
    let net = NetSettings { request_timeout: Duration::from_millis(300), max_retries: 0, ..NetSettings::default() };
    let (base, _) = server(vec![None, None]);
    let started = Instant::now();
    let c = provider(&base, net).stream(req(), no_wake()).collect();
    assert!(matches!(c.error, Some(AiError::Timeout(_))), "{:?}", c.error);
    assert!(started.elapsed() < Duration::from_secs(10));

    let slow = NetSettings { request_timeout: Duration::from_secs(30), ..NetSettings::default() };
    let p = provider(&base, slow);
    let h = p.stream(req(), no_wake());
    std::thread::sleep(Duration::from_millis(100));
    h.cancel();
    let started = Instant::now();
    let ev = h.rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(ev, Event::Error(AiError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(5));
}
