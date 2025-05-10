use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::collections::HashMap;

#[derive(Serialize_repr, Deserialize_repr, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TunnelMessageType {
    TunnelCreated = 0,
    TunnelDestroyed = 1,
    TunnelRequest = 2,
    TunnelResponse = 3,
    TunnelAuthRequest = 4,
    TunnelAuthResponse = 5,
    TunnelAuthFailure = 6,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TunnelMessage {
    #[serde(rename = "type")]
    pub message_type: TunnelMessageType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelStatus {
    Disconnected,
    Connecting,
    Authenticating,
    Establishing,
    Connected,
    Reconnecting, // Future use
    Error,
}

pub const HEADER_LOCAL_URL: &str = "Local-URL";
pub const HEADER_PROD_URL: &str = "Prod-URL";
pub const HEADER_X_STATUS_CODE: &str = "X-Status-Code";
pub const HEADER_CONTENT_TYPE: &str = "Content-Type";
pub const HEADER_HOST: &str = "Host";
pub const HEADER_X_FORWARDED_HOST: &str = "X-Forwarded-Host";
