use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
pub const ACTIVATION_FLAG: &str = "--culsans-activate";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationOrigin {
    pub platform: Option<String>,
    pub foreground_hwnd: Option<String>,
    pub foreground_pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivationEnvelope {
    pub version: u32,
    pub request_id: String,
    pub action: String,
    pub origin: Option<ActivationOrigin>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct QuickInsertPayload {
    pub query: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveSnippetPayload {
    pub name: String,
    pub content: String,
    pub group_name: Option<String>,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("activation payload is not valid base64url")]
    InvalidEncoding,
    #[error("activation payload is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("unsupported activation protocol version {0}")]
    UnsupportedVersion(u32),
    #[error("unsupported Echo activation action {0}")]
    UnknownAction(String),
    #[error("activation payload is invalid: {0}")]
    InvalidPayload(String),
}

pub fn new_envelope(action: impl Into<String>, payload: Value) -> ActivationEnvelope {
    ActivationEnvelope {
        version: PROTOCOL_VERSION,
        request_id: Uuid::new_v4().to_string(),
        action: action.into(),
        origin: Some(ActivationOrigin {
            platform: Some("windows".to_owned()),
            foreground_hwnd: None,
            foreground_pid: None,
        }),
        payload,
    }
}

pub fn encode(envelope: &ActivationEnvelope) -> Result<String, ProtocolError> {
    let bytes = serde_json::to_vec(envelope)
        .map_err(|error| ProtocolError::InvalidJson(error.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub fn decode(argument: &str) -> Result<ActivationEnvelope, ProtocolError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(argument)
        .map_err(|_| ProtocolError::InvalidEncoding)?;
    let envelope: ActivationEnvelope = serde_json::from_slice(&bytes)
        .map_err(|error| ProtocolError::InvalidJson(error.to_string()))?;
    validate(&envelope)?;
    Ok(envelope)
}

pub fn decode_args<'a>(
    args: impl IntoIterator<Item = &'a str>,
) -> Option<Result<ActivationEnvelope, ProtocolError>> {
    let args = args.into_iter().collect::<Vec<_>>();
    args.windows(2)
        .find(|window| window[0] == ACTIVATION_FLAG)
        .map(|window| decode(window[1]))
}

pub fn validate(envelope: &ActivationEnvelope) -> Result<(), ProtocolError> {
    if envelope.version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(envelope.version));
    }
    match envelope.action.as_str() {
        "echo.open" | "echo.quick_insert" | "echo.settings" => Ok(()),
        "echo.save_snippet" => {
            serde_json::from_value::<SaveSnippetPayload>(envelope.payload.clone())
                .map(|_| ())
                .map_err(|error| ProtocolError::InvalidPayload(error.to_string()))
        }
        action => Err(ProtocolError::UnknownAction(action.to_owned())),
    }
}

pub fn quick_insert_payload(
    envelope: &ActivationEnvelope,
) -> Result<QuickInsertPayload, ProtocolError> {
    if envelope.action != "echo.quick_insert" {
        return Err(ProtocolError::InvalidPayload(
            "action is not echo.quick_insert".to_owned(),
        ));
    }
    serde_json::from_value(envelope.payload.clone())
        .map_err(|error| ProtocolError::InvalidPayload(error.to_string()))
}

pub fn save_snippet_payload(
    envelope: &ActivationEnvelope,
) -> Result<SaveSnippetPayload, ProtocolError> {
    if envelope.action != "echo.save_snippet" {
        return Err(ProtocolError::InvalidPayload(
            "action is not echo.save_snippet".to_owned(),
        ));
    }
    serde_json::from_value(envelope.payload.clone())
        .map_err(|error| ProtocolError::InvalidPayload(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn activation_round_trips_through_url_safe_json() {
        let envelope = new_envelope("echo.quick_insert", json!({"query": "hello / world"}));
        let encoded = encode(&envelope).unwrap();
        assert!(!encoded.contains('+'));
        assert_eq!(decode(&encoded).unwrap(), envelope);
        assert_eq!(
            quick_insert_payload(&envelope).unwrap().query.as_deref(),
            Some("hello / world")
        );
    }

    #[test]
    fn unknown_versions_and_actions_fail_closed() {
        let mut envelope = new_envelope("echo.open", json!({}));
        envelope.version = 2;
        assert!(matches!(
            validate(&envelope),
            Err(ProtocolError::UnsupportedVersion(2))
        ));
        envelope.version = 1;
        envelope.action = "echo.unknown".to_owned();
        assert!(matches!(
            validate(&envelope),
            Err(ProtocolError::UnknownAction(_))
        ));
    }

    #[test]
    fn save_snippet_requires_a_small_structured_payload() {
        let envelope = new_envelope(
            "echo.save_snippet",
            json!({"name": "Greeting", "content": "Hello", "group_name": "Common"}),
        );
        assert_eq!(save_snippet_payload(&envelope).unwrap().name, "Greeting");
        let invalid = new_envelope("echo.save_snippet", json!({"content": "Hello"}));
        assert!(validate(&invalid).is_err());
    }
}
