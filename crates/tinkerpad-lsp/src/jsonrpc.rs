//! JSON-RPC messages, kept as JSON.
//!
//! Messages stay `serde_json::Value` at this layer. The proxy (02-03) reaches
//! for `lsp-types` only on the handful of methods it actually inspects: a
//! server may send extensions this core has never heard of, and it must be able
//! to pass those through untouched.
//!
//! Classification follows the JSON-RPC 2.0 spec: `method` and `id` together are
//! a request, `method` alone a notification, `id` alone a response.

use serde_json::{Map, Value, json};

/// A message on the wire, in either direction.
#[derive(Clone, Debug, PartialEq)]
pub struct RawMessage(Value);

/// What a message is, by the fields it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Request,
    Response,
    Notification,
    /// Neither a method nor an id: nothing can be done with it but log it.
    Unknown,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("message is not JSON: {source}")]
    NotJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("message is not a JSON object")]
    NotAnObject,
}

impl RawMessage {
    /// Wrap a value that is already a message.
    pub fn new(value: Value) -> Self {
        Self(value)
    }

    /// Parse a body from the framing layer.
    pub fn from_slice(body: &[u8]) -> Result<Self, ParseError> {
        let value: Value =
            serde_json::from_slice(body).map_err(|source| ParseError::NotJson { source })?;
        if !value.is_object() {
            return Err(ParseError::NotAnObject);
        }
        Ok(Self(value))
    }

    /// A request, with the `jsonrpc` field filled in.
    pub fn request(id: impl Into<Value>, method: &str, params: Value) -> Self {
        Self(json!({ "jsonrpc": "2.0", "id": id.into(), "method": method, "params": params }))
    }

    /// A notification: a request with no id, so no reply is expected.
    pub fn notification(method: &str, params: Value) -> Self {
        Self(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    /// A successful response to the request with this id.
    pub fn response(id: impl Into<Value>, result: Value) -> Self {
        Self(json!({ "jsonrpc": "2.0", "id": id.into(), "result": result }))
    }

    /// A failed response. `code` is a JSON-RPC error code.
    pub fn error(id: impl Into<Value>, code: i64, message: &str) -> Self {
        Self(json!({
            "jsonrpc": "2.0",
            "id": id.into(),
            "error": { "code": code, "message": message },
        }))
    }

    pub fn kind(&self) -> MessageKind {
        match (self.method().is_some(), self.id().is_some()) {
            (true, true) => MessageKind::Request,
            (true, false) => MessageKind::Notification,
            (false, true) => MessageKind::Response,
            (false, false) => MessageKind::Unknown,
        }
    }

    pub fn is_request(&self) -> bool {
        self.kind() == MessageKind::Request
    }

    pub fn is_response(&self) -> bool {
        self.kind() == MessageKind::Response
    }

    pub fn is_notification(&self) -> bool {
        self.kind() == MessageKind::Notification
    }

    /// The `id`, which JSON-RPC allows to be a number or a string. A null id
    /// (what a server sends when it can't tell which request failed) counts as
    /// no id, since nothing can be routed by it.
    pub fn id(&self) -> Option<&Value> {
        match self.0.get("id") {
            Some(Value::Null) | None => None,
            Some(id) => Some(id),
        }
    }

    pub fn method(&self) -> Option<&str> {
        self.0.get("method").and_then(Value::as_str)
    }

    pub fn params(&self) -> Option<&Value> {
        self.0.get("params")
    }

    pub fn result(&self) -> Option<&Value> {
        self.0.get("result")
    }

    /// The `error` object of a failed response, if this is one.
    pub fn error_object(&self) -> Option<&Value> {
        self.0.get("error")
    }

    /// The message as JSON.
    pub fn value(&self) -> &Value {
        &self.0
    }

    pub fn into_value(self) -> Value {
        self.0
    }

    /// The object's fields, for the proxy's URI rewriting (02-03).
    pub fn object_mut(&mut self) -> Option<&mut Map<String, Value>> {
        self.0.as_object_mut()
    }

    /// The bytes to frame and send.
    pub fn to_vec(&self) -> Vec<u8> {
        serde_json::to_vec(&self.0).expect("a Value always serialises")
    }
}

impl std::fmt::Display for RawMessage {
    /// One line, for logs: the kind and what it is about, never the whole body.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind() {
            MessageKind::Request => write!(
                f,
                "request {} id={}",
                self.method().unwrap_or_default(),
                self.id().unwrap_or(&Value::Null)
            ),
            MessageKind::Notification => {
                write!(f, "notification {}", self.method().unwrap_or_default())
            }
            MessageKind::Response => {
                let outcome = if self.error_object().is_some() {
                    "error"
                } else {
                    "ok"
                };
                write!(
                    f,
                    "response id={} {outcome}",
                    self.id().unwrap_or(&Value::Null)
                )
            }
            MessageKind::Unknown => write!(f, "unknown message"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_method_and_id() {
        let request = RawMessage::request(1, "initialize", json!({}));
        assert_eq!(request.kind(), MessageKind::Request);
        assert!(request.is_request() && !request.is_response() && !request.is_notification());
        assert_eq!(request.method(), Some("initialize"));
        assert_eq!(request.id(), Some(&json!(1)));

        let notification = RawMessage::notification("initialized", json!({}));
        assert!(notification.is_notification());
        assert_eq!(notification.id(), None);

        let response = RawMessage::response(1, json!({ "capabilities": {} }));
        assert!(response.is_response());
        assert_eq!(response.method(), None);
        assert_eq!(response.result(), Some(&json!({ "capabilities": {} })));

        let error = RawMessage::error(2, -32601, "method not found");
        assert!(error.is_response());
        assert_eq!(error.error_object().unwrap()["code"], json!(-32601));
    }

    #[test]
    fn a_string_id_works_like_a_number_one() {
        let request = RawMessage::request("abc", "test/echo", json!([]));
        assert!(request.is_request());
        assert_eq!(request.id(), Some(&json!("abc")));
    }

    #[test]
    fn a_null_id_is_no_id() {
        let message = RawMessage::new(json!({ "jsonrpc": "2.0", "id": null, "error": {} }));
        assert_eq!(message.id(), None);
        assert_eq!(message.kind(), MessageKind::Unknown);
    }

    #[test]
    fn round_trips_through_bytes_unchanged() {
        let original = RawMessage::request(7, "textDocument/hover", json!({ "x": [1, 2, 3] }));
        let parsed = RawMessage::from_slice(&original.to_vec()).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn keeps_fields_it_does_not_understand() {
        let body = br#"{"jsonrpc":"2.0","method":"rust-analyzer/serverStatus","params":{"quiescent":true},"extra":1}"#;
        let message = RawMessage::from_slice(body).unwrap();
        assert!(message.is_notification());
        assert_eq!(message.value()["extra"], json!(1));
        assert_eq!(
            String::from_utf8(message.to_vec()).unwrap(),
            String::from_utf8(body.to_vec()).unwrap()
        );
    }

    #[test]
    fn rejects_what_is_not_an_object() {
        assert!(matches!(
            RawMessage::from_slice(b"[1,2]"),
            Err(ParseError::NotAnObject)
        ));
        assert!(matches!(
            RawMessage::from_slice(b"{oops"),
            Err(ParseError::NotJson { .. })
        ));
    }

    #[test]
    fn describes_itself_for_the_log() {
        assert_eq!(
            RawMessage::request(1, "initialize", json!({})).to_string(),
            "request initialize id=1"
        );
        assert_eq!(
            RawMessage::notification("exit", json!(null)).to_string(),
            "notification exit"
        );
        assert_eq!(
            RawMessage::error(3, -1, "no").to_string(),
            "response id=3 error"
        );
    }
}
