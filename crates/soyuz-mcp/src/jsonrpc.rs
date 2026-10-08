//! JSON-RPC 2.0 framing helpers and the MCP error codes.

use serde_json::{Value, json};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// Legacy (≤ 2025-11-25) "resource not found".
pub const LEGACY_RESOURCE_NOT_FOUND: i64 = -32002;
/// 2026-07-28: HTTP headers do not match the body.
pub const HEADER_MISMATCH: i64 = -32020;
/// 2026-07-28: a capability the request needs was not declared by the client.
pub const MISSING_CLIENT_CAPABILITY: i64 = -32021;
/// 2026-07-28: the requested protocol version is not supported.
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// A JSON-RPC error object.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into(), data: None }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        RpcError::new(INVALID_PARAMS, message)
    }

    pub fn to_value(&self) -> Value {
        let mut e = json!({"code": self.code, "message": self.message});
        if let Some(d) = &self.data {
            e["data"] = d.clone();
        }
        e
    }
}

/// What kind of message a parsed value is.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// A response (we never send requests as a server, so these are ignored).
    Response,
    Invalid(RpcError, Value),
}

/// Classify one decoded message.
pub fn classify(v: Value) -> Incoming {
    let Some(o) = v.as_object() else {
        return Incoming::Invalid(RpcError::new(INVALID_REQUEST, "message must be a JSON object"), Value::Null);
    };
    let id = o.get("id").cloned();
    if o.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Incoming::Invalid(RpcError::new(INVALID_REQUEST, "jsonrpc must be \"2.0\""), id.unwrap_or_default());
    }
    if o.contains_key("result") || o.contains_key("error") {
        return Incoming::Response;
    }
    let Some(method) = o.get("method").and_then(Value::as_str) else {
        return Incoming::Invalid(RpcError::new(INVALID_REQUEST, "missing method"), id.unwrap_or_default());
    };
    let params = o.get("params").cloned().unwrap_or(Value::Null);
    if !(params.is_null() || params.is_object()) {
        return Incoming::Invalid(RpcError::invalid_params("params must be an object"), id.unwrap_or_default());
    }
    match id {
        None => Incoming::Notification { method: method.to_string(), params },
        Some(id @ (Value::String(_) | Value::Number(_))) => {
            Incoming::Request { id, method: method.to_string(), params }
        }
        Some(other) => Incoming::Invalid(RpcError::new(INVALID_REQUEST, "id must be a string or an integer"), other),
    }
}

pub fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

pub fn error(id: &Value, err: &RpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": err.to_value()})
}

pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

pub fn request(id: &Value, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// A stable string key for an id (`1` and `"1"` stay distinct).
pub fn id_key(id: &Value) -> String {
    id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        let r = classify(json!({"jsonrpc":"2.0","id":1,"method":"ping"}));
        assert!(matches!(r, Incoming::Request { ref method, .. } if method == "ping"));
        let n = classify(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        assert!(matches!(n, Incoming::Notification { .. }));
        assert_eq!(classify(json!({"jsonrpc":"2.0","id":1,"result":{}})), Incoming::Response);
        assert!(matches!(classify(json!([1])), Incoming::Invalid(..)));
        assert!(matches!(classify(json!({"jsonrpc":"1.0","id":1,"method":"x"})), Incoming::Invalid(..)));
        assert!(matches!(classify(json!({"jsonrpc":"2.0","id":null,"method":"x"})), Incoming::Invalid(..)));
        assert!(matches!(classify(json!({"jsonrpc":"2.0","id":1,"method":"x","params":[1]})), Incoming::Invalid(..)));
    }
}
