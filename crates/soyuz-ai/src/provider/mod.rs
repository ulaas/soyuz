//! Provider layer: one [`Provider`] per configured profile, backed by a
//! wire-format [`Backend`] (Anthropic Messages, OpenAI Chat Completions, OpenAI Responses,
//! Gemini). The generic part here does HTTP, retries (429 / overloaded / 5xx with
//! `retry-after`), timeouts, cancellation and SSE decoding; backends only encode requests
//! and decode stream events, so they are tested offline against recorded fixtures.

pub mod anthropic;
pub mod gemini;
pub mod openai_chat;
pub mod openai_responses;
pub mod probe;

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;
use soyuz_core::config::{Ai, AiProfile, AiProviderKind, AiToolMode};

use crate::credentials::{self, Secret};
use crate::error::{AiError, AiResult};
use crate::json_tools;
use crate::runtime::{CancelToken, Wake, client, runtime};
use crate::sse::{SseEvent, SseParser};
use crate::types::{ChatRequest, Event, ToolProtocol, Wire};

pub use probe::ProbeReport;

/// HTTP method of a [`HttpPlan`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// A fully built HTTP request (secrets included, so it is never logged).
#[derive(Clone)]
pub struct HttpPlan {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
}

impl std::fmt::Debug for HttpPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("HttpPlan")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &names)
            .finish()
    }
}

/// Stateful decoder of one streamed response.
pub trait Decoder: Send {
    /// Decode one SSE event into zero or more [`Event`]s.
    fn event(&mut self, ev: &SseEvent) -> AiResult<Vec<Event>>;
    /// End of the byte stream: flush pending tool calls and emit the terminal event if the
    /// stream did not carry one.
    fn finish(&mut self) -> Vec<Event>;
}

/// Static description of how a backend is reached.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub base_url: String,
    pub key: Option<Secret>,
    pub headers: Vec<(String, String)>,
}

/// A wire format.
pub trait Backend: Send + Sync {
    fn wire(&self) -> Wire;
    /// Build the streaming chat request.
    fn chat(&self, ep: &Endpoint, req: &ChatRequest) -> AiResult<HttpPlan>;
    fn decoder(&self) -> Box<dyn Decoder>;
    /// `GET` request listing models (test connection).
    fn models(&self, ep: &Endpoint) -> HttpPlan;
    fn parse_models(&self, v: &Value) -> Vec<String>;
}

/// Network limits for a provider.
#[derive(Clone, Copy, Debug)]
pub struct NetSettings {
    pub request_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_retries: u32,
    /// Longest `retry-after` we are willing to wait; longer ones fail immediately.
    pub max_retry_wait: Duration,
}

impl Default for NetSettings {
    fn default() -> Self {
        NetSettings {
            request_timeout: Duration::from_secs(60),
            idle_timeout: Duration::from_secs(120),
            max_retries: 3,
            max_retry_wait: Duration::from_secs(60),
        }
    }
}

impl NetSettings {
    pub fn from_config(ai: &Ai) -> Self {
        NetSettings {
            request_timeout: Duration::from_secs(ai.request_timeout_secs.max(1)),
            idle_timeout: Duration::from_secs(ai.idle_timeout_secs.max(1)),
            max_retries: ai.max_retries,
            ..NetSettings::default()
        }
    }
}

/// A running streamed request. Dropping it cancels the request.
pub struct StreamHandle {
    pub rx: crossbeam_channel::Receiver<Event>,
    cancel: CancelToken,
}

impl StreamHandle {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// Block until the stream ends, collecting it (tests, workers; never the UI thread).
    pub fn collect(self) -> crate::types::Collected {
        let mut c = crate::types::Collected::default();
        while let Ok(ev) = self.rx.recv() {
            let term = ev.is_terminal();
            c.push(ev);
            if term {
                break;
            }
        }
        c
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// One configured model provider.
pub struct Provider {
    pub name: String,
    pub kind: AiProviderKind,
    pub model: String,
    pub tool_mode: AiToolMode,
    pub max_tokens: u32,
    endpoint: Endpoint,
    backend: Box<dyn Backend>,
    net: NetSettings,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("model", &self.model)
            .finish()
    }
}

/// The backend for a provider kind.
pub fn backend_for(kind: AiProviderKind) -> Box<dyn Backend> {
    match kind {
        AiProviderKind::Anthropic => Box::new(anthropic::Anthropic),
        AiProviderKind::OpenaiResponses => Box::new(openai_responses::OpenAiResponses),
        AiProviderKind::Gemini => Box::new(gemini::Gemini),
        AiProviderKind::Openai => Box::new(openai_chat::OpenAiChat { max_completion_tokens: true }),
        AiProviderKind::Ollama
        | AiProviderKind::LmStudio
        | AiProviderKind::LlamaCpp
        | AiProviderKind::Vllm
        | AiProviderKind::Localai
        | AiProviderKind::Custom => Box::new(openai_chat::OpenAiChat { max_completion_tokens: false }),
    }
}

impl Provider {
    /// Build from a config profile, resolving its key reference (credential store or
    /// environment). Touches the credential store: call off the UI thread.
    pub fn from_profile(profile: &AiProfile, net: NetSettings) -> AiResult<Provider> {
        let key = credentials::resolve(&profile.key)?;
        Self::with_key(profile, key, net)
    }

    /// Build with an explicit key (tests, a key typed in the settings page).
    pub fn with_key(profile: &AiProfile, key: Option<Secret>, net: NetSettings) -> AiResult<Provider> {
        let base = if profile.base_url.trim().is_empty() {
            profile.kind.default_base_url().to_string()
        } else {
            profile.base_url.trim().to_string()
        };
        if base.is_empty() {
            return Err(AiError::Config(format!("profile “{}” needs a base URL", profile.name)));
        }
        if !(base.starts_with("https://") || base.starts_with("http://")) {
            return Err(AiError::Config(format!("base URL must start with http:// or https:// ({base})")));
        }
        if key.is_none() && !profile.kind.is_local() && profile.kind != AiProviderKind::Custom {
            return Err(AiError::Config(format!("profile “{}” has no API key", profile.name)));
        }
        Ok(Provider {
            name: profile.name.clone(),
            kind: profile.kind,
            model: profile.model.clone(),
            tool_mode: profile.tool_mode,
            max_tokens: profile.max_tokens,
            endpoint: Endpoint {
                base_url: base.trim_end_matches('/').to_string(),
                key,
                headers: profile.headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            },
            backend: backend_for(profile.kind),
            net,
        })
    }

    pub fn wire(&self) -> Wire {
        self.backend.wire()
    }

    /// Start a streamed chat. Events arrive on the handle's channel; `wake` runs after each.
    /// With [`ToolProtocol::Json`] the tools are described in the system prompt and the
    /// reply is parsed (and schema-checked) into `ToolCall` events.
    pub fn stream(&self, req: ChatRequest, wake: Wake) -> StreamHandle {
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = CancelToken::new();
        let (req, json_tools) = match req.tool_protocol {
            ToolProtocol::Json if !req.tools.is_empty() => {
                let tools = req.tools.clone();
                (json_tools::lower_request(req), Some(tools))
            }
            _ => (req, None),
        };
        let plan = self.backend.chat(&self.endpoint, &req);
        let mut decoder = self.backend.decoder();
        if let Some(tools) = json_tools {
            decoder = Box::new(json_tools::JsonToolDecoder::new(decoder, tools));
        }
        let net = self.net;
        let token = cancel.clone();
        match plan {
            Ok(plan) => {
                runtime().spawn(async move {
                    run_stream(plan, decoder, net, token, tx, wake).await;
                });
            }
            Err(e) => {
                let _ = tx.send(Event::Error(e));
                wake();
            }
        }
        StreamHandle { rx, cancel }
    }

    /// List the provider's models (async; result on the returned channel).
    pub fn list_models(&self, wake: Wake) -> crossbeam_channel::Receiver<AiResult<Vec<String>>> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let plan = self.backend.models(&self.endpoint);
        let net = self.net;
        let parse = self.backend_parse_models();
        runtime().spawn(async move {
            let r = fetch_json(&plan, net).await.map(|v| parse(&v));
            let _ = tx.send(r);
            wake();
        });
        rx
    }

    fn backend_parse_models(&self) -> impl Fn(&Value) -> Vec<String> + Send + 'static {
        let backend = backend_for(self.kind);
        move |v| backend.parse_models(v)
    }

    /// Test connection + capability probe (see [`probe::run`]).
    pub fn probe(self: &std::sync::Arc<Self>, wake: Wake) -> crossbeam_channel::Receiver<ProbeReport> {
        probe::spawn(self.clone(), wake)
    }
}

/// Decode a complete recorded SSE body with a backend's decoder (fixture tests, replay).
pub fn decode_recorded(decoder: &mut dyn Decoder, body: &[u8]) -> Vec<Event> {
    let mut out = Vec::new();
    let mut parser = SseParser::new();
    let mut evs = parser.feed(body);
    evs.extend(parser.finish());
    for ev in evs {
        match decoder.event(&ev) {
            Ok(v) => {
                let term = v.iter().any(Event::is_terminal);
                out.extend(v);
                if term {
                    return out;
                }
            }
            Err(e) => {
                out.push(Event::Error(e));
                return out;
            }
        }
    }
    out.extend(decoder.finish());
    out
}

fn build(plan: &HttpPlan) -> AiResult<reqwest::RequestBuilder> {
    let c = client()?;
    let mut rb = match plan.method {
        Method::Get => c.get(&plan.url),
        Method::Post => c.post(&plan.url),
    };
    for (k, v) in &plan.headers {
        rb = rb.header(k.as_str(), v.as_str());
    }
    if let Some(body) = &plan.body {
        rb = rb.json(body);
    }
    Ok(rb)
}

/// Send with retries; returns the successful response.
async fn send_with_retries(plan: &HttpPlan, net: NetSettings, cancel: &CancelToken) -> AiResult<reqwest::Response> {
    let mut attempt = 0u32;
    loop {
        let rb = build(plan)?;
        let res = tokio::select! {
            _ = cancel.cancelled() => return Err(AiError::Cancelled),
            r = tokio::time::timeout(net.request_timeout, rb.send()) => r,
        };
        let err = match res {
            Err(_) => AiError::Timeout(format!("no response within {}s", net.request_timeout.as_secs())),
            Ok(Err(e)) => network_error(&e),
            Ok(Ok(resp)) if resp.status().is_success() => return Ok(resp),
            Ok(Ok(resp)) => error_from_response(resp).await,
        };
        if !err.is_retryable() || attempt >= net.max_retries {
            return Err(err);
        }
        let wait = retry_delay(&err, attempt);
        if wait > net.max_retry_wait {
            return Err(err);
        }
        log::info!("soyuz-ai: retrying in {:.1}s after {err}", wait.as_secs_f32());
        tokio::select! {
            _ = cancel.cancelled() => return Err(AiError::Cancelled),
            _ = tokio::time::sleep(wait) => {}
        }
        attempt += 1;
    }
}

/// Delay before retry `attempt` (0-based): `retry-after` when given, else exponential
/// backoff (1, 2, 4 … s, capped at 30 s).
pub fn retry_delay(err: &AiError, attempt: u32) -> Duration {
    if let AiError::RateLimited { retry_after: Some(d) } = err {
        return *d;
    }
    Duration::from_millis((1000u64 << attempt.min(5)).min(30_000))
}

fn network_error(e: &reqwest::Error) -> AiError {
    if e.is_timeout() { AiError::Timeout(e.to_string()) } else { AiError::Network(e.to_string()) }
}

/// Parse a `retry-after` header value: delta seconds or an HTTP date.
pub fn parse_retry_after(v: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
    let v = v.trim();
    if let Ok(secs) = v.parse::<f64>() {
        return (secs >= 0.0).then(|| Duration::from_secs_f64(secs));
    }
    let t = chrono::DateTime::parse_from_rfc2822(v).ok()?;
    let d = t.with_timezone(&chrono::Utc) - now;
    Some(d.to_std().unwrap_or(Duration::ZERO))
}

/// Map an error status + body to an [`AiError`].
pub fn classify_status(status: u16, retry_after: Option<Duration>, body: &str) -> AiError {
    let message = error_message(body).unwrap_or_else(|| body.chars().take(300).collect());
    match status {
        401 | 403 => AiError::Auth { status, message },
        429 => AiError::RateLimited { retry_after },
        408 | 500 | 502 | 503 | 504 | 529 => AiError::Overloaded(status),
        _ => AiError::Http { status, message },
    }
}

/// `{"error": {"message": …}}` (Anthropic / OpenAI / Gemini) or `{"message": …}`.
pub(crate) fn error_message(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let e = v.get("error").unwrap_or(&v);
    if let Some(s) = e.as_str() {
        return Some(s.to_string());
    }
    e.get("message").and_then(Value::as_str).map(str::to_string)
}

async fn error_from_response(resp: reqwest::Response) -> AiError {
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_retry_after(v, chrono::Utc::now()));
    let body = match tokio::time::timeout(Duration::from_secs(10), resp.text()).await {
        Ok(Ok(t)) => t,
        _ => String::new(),
    };
    classify_status(status, retry_after, &body)
}

async fn fetch_json(plan: &HttpPlan, net: NetSettings) -> AiResult<Value> {
    let cancel = CancelToken::new();
    let resp = send_with_retries(plan, net, &cancel).await?;
    let text = tokio::time::timeout(net.idle_timeout, resp.text())
        .await
        .map_err(|_| AiError::Timeout("reading response".into()))?
        .map_err(|e| network_error(&e))?;
    serde_json::from_str(&text).map_err(|e| AiError::Decode(e.to_string()))
}

async fn run_stream(
    plan: HttpPlan,
    mut decoder: Box<dyn Decoder>,
    net: NetSettings,
    cancel: CancelToken,
    tx: crossbeam_channel::Sender<Event>,
    wake: Wake,
) {
    let send = |ev: Event| {
        let _ = tx.send(ev);
        wake();
    };
    let resp = match send_with_retries(&plan, net, &cancel).await {
        Ok(r) => r,
        Err(e) => return send(Event::Error(e)),
    };
    let started = Instant::now();
    let mut body = resp.bytes_stream();
    let mut parser = SseParser::new();
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return send(Event::Error(AiError::Cancelled)),
            n = tokio::time::timeout(net.idle_timeout, body.next()) => n,
        };
        let chunk = match next {
            Err(_) => {
                return send(Event::Error(AiError::Timeout(format!(
                    "stream idle for {}s",
                    net.idle_timeout.as_secs()
                ))));
            }
            Ok(None) => break,
            Ok(Some(Err(e))) => return send(Event::Error(network_error(&e))),
            Ok(Some(Ok(b))) => b,
        };
        for ev in parser.feed(&chunk) {
            if deliver(decoder.as_mut(), &ev, &send) {
                return;
            }
        }
    }
    if let Some(ev) = parser.finish()
        && deliver(decoder.as_mut(), &ev, &send)
    {
        return;
    }
    for ev in decoder.finish() {
        send(ev);
    }
    log::debug!("soyuz-ai: stream finished in {:.1}s", started.elapsed().as_secs_f32());
}

/// Decode one SSE event and send the results; `true` once a terminal event went out.
fn deliver(decoder: &mut dyn Decoder, ev: &SseEvent, send: &impl Fn(Event)) -> bool {
    match decoder.event(ev) {
        Ok(evs) => {
            let mut term = false;
            for e in evs {
                term |= e.is_terminal();
                send(e);
            }
            term
        }
        Err(e) => {
            send(Event::Error(e));
            true
        }
    }
}

/// Parse a JSON tool-argument string; empty means `{}`.
pub(crate) fn parse_args(raw: &str) -> AiResult<Value> {
    if raw.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(raw).map_err(|e| AiError::Decode(format!("tool arguments are not valid JSON: {e}")))
}

/// Push `Authorization: Bearer <key>` when there is a key.
pub(crate) fn bearer(ep: &Endpoint, headers: &mut Vec<(String, String)>) {
    if let Some(k) = &ep.key {
        headers.push(("authorization".into(), format!("Bearer {}", k.expose())));
    }
}

/// Base headers: JSON content type plus the profile's extra headers.
pub(crate) fn base_headers(ep: &Endpoint) -> Vec<(String, String)> {
    let mut h = vec![("content-type".to_string(), "application/json".to_string())];
    h.extend(ep.headers.iter().cloned());
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_and_status_mapping() {
        let now = chrono::DateTime::parse_from_rfc3339("2015-10-21T07:28:00Z").unwrap().with_timezone(&chrono::Utc);
        assert_eq!(parse_retry_after("7", now), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:30 GMT", now), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after("soon", now), None);
        let e = classify_status(429, Some(Duration::from_secs(3)), "{}");
        assert!(e.is_retryable());
        assert_eq!(retry_delay(&e, 0), Duration::from_secs(3));
        assert_eq!(retry_delay(&AiError::Overloaded(529), 2), Duration::from_secs(4));
        assert!(matches!(
            classify_status(401, None, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#),
            AiError::Auth { status: 401, ref message } if message == "invalid x-api-key"
        ));
        assert!(matches!(classify_status(400, None, "plain"), AiError::Http { status: 400, .. }));
        assert!(classify_status(529, None, "").is_retryable());
    }

    #[test]
    fn profile_validation() {
        let mut p = AiProfile { name: "x".into(), kind: AiProviderKind::Anthropic, ..Default::default() };
        assert!(matches!(Provider::with_key(&p, None, NetSettings::default()), Err(AiError::Config(_))));
        p.kind = AiProviderKind::Ollama;
        let prov = Provider::with_key(&p, None, NetSettings::default()).unwrap();
        assert_eq!(prov.endpoint.base_url, "http://localhost:11434/v1");
        p.base_url = "ftp://x".into();
        assert!(Provider::with_key(&p, None, NetSettings::default()).is_err());
    }

    #[test]
    fn plan_debug_hides_secrets() {
        let plan = HttpPlan {
            method: Method::Post,
            url: "https://x".into(),
            headers: vec![("x-api-key".into(), "sk-secret".into())],
            body: None,
        };
        assert!(!format!("{plan:?}").contains("sk-secret"));
    }
}
