use crate::error::TunnelError;
use crate::types::TunnelMessage;
use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TunnelConfig {
    pub local_port: String,
    pub auth_timeout: Duration,
    pub request_timeout: Duration,
    pub response_timeout: Duration,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        TunnelConfig {
            local_port: String::new(),
            auth_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(20),
            response_timeout: Duration::from_secs(20),
        }
    }
}

pub type OnAuthCallback = Box<dyn Fn(&str) + Send + Sync + 'static>;
pub type OnConnectedCallback = Box<dyn Fn(&str, &str, &str, &str) + Send + Sync + 'static>;
pub type OnDisconnectedCallback = Box<dyn Fn() + Send + Sync + 'static>;
pub type OnErrorCallback = Box<dyn Fn(&TunnelError) + Send + Sync + 'static>;
pub type OnRequestCallback = Box<dyn Fn(&TunnelMessage) + Send + Sync + 'static>;
pub type OnSendingResponseCallback =
    Box<dyn Fn(&TunnelMessage, StatusCode, &HeaderMap, &[u8]) + Send + Sync + 'static>;

#[derive(Clone)]
pub struct SDKConfig {
    pub tunnel_server: String,
    pub auth_token: String,

    pub on_auth: Option<Arc<OnAuthCallback>>,
    pub on_connected: Option<Arc<OnConnectedCallback>>,
    pub on_disconnected: Option<Arc<OnDisconnectedCallback>>,
    pub on_error: Option<Arc<OnErrorCallback>>,
    pub on_request: Option<Arc<OnRequestCallback>>,
    pub on_sending_response: Option<Arc<OnSendingResponseCallback>>,
}

impl Default for SDKConfig {
    fn default() -> Self {
        SDKConfig {
            tunnel_server: "tunnel.ngorok.site:9000".to_string(),
            auth_token: String::new(),
            on_auth: Some(Arc::new(Box::new(|_token| {
                tracing::info!("Attempting authentication...");
            }))),
            on_connected: Some(Arc::new(Box::new(
                |local_port, local_url, prod_url, tunnel_id| {
                    tracing::info!(tunnel_id, local_url, prod_url, "Tunnel established!");
                    tracing::info!("Forwarding from http://localhost:{}", local_port);
                },
            ))),
            on_disconnected: Some(Arc::new(Box::new(|| {
                tracing::info!("Tunnel disconnected");
            }))),
            on_error: Some(Arc::new(Box::new(|err| {
                tracing::error!("Error: {:?}", err);
            }))),
            on_request: Some(Arc::new(Box::new(|msg| {
                tracing::info!(
                    id = msg.id.as_deref().unwrap_or("unknown"),
                    method = msg.method.as_deref().unwrap_or("unknown"),
                    path = msg.path.as_deref().unwrap_or("unknown"),
                    "Received request"
                );
            }))),
            on_sending_response: Some(Arc::new(Box::new(|msg, status, _headers, body| {
                tracing::info!(
                    id = msg.id.as_deref().unwrap_or("unknown"),
                    status = status.as_u16(),
                    path = msg.path.as_deref().unwrap_or("unknown"),
                    body_len = body.len(),
                    "Sending response for request"
                );
            }))),
        }
    }
}

impl SDKConfig {
    pub(crate) fn trigger_on_auth(&self) {
        if let Some(cb) = &self.on_auth {
            cb(&self.auth_token);
        }
    }

    pub(crate) fn trigger_on_connected(
        &self,
        local_port: &str,
        local_url: &str,
        prod_url: &str,
        tunnel_id: &str,
    ) {
        if let Some(cb) = &self.on_connected {
            cb(local_port, local_url, prod_url, tunnel_id);
        }
    }

    pub(crate) fn trigger_on_disconnected(&self) {
        if let Some(cb) = &self.on_disconnected {
            cb();
        }
    }

    pub(crate) fn trigger_on_error(&self, err: &TunnelError) {
        if let Some(cb) = &self.on_error {
            cb(err);
        }
    }

    pub(crate) fn trigger_on_request(&self, msg: &TunnelMessage) {
        if let Some(cb) = &self.on_request {
            cb(msg);
        }
    }

    pub(crate) fn trigger_on_sending_response(
        &self,
        msg: &TunnelMessage,
        status: StatusCode,
        headers: &HeaderMap,
        body: &[u8],
    ) {
        if let Some(cb) = &self.on_sending_response {
            cb(msg, status, headers, body);
        }
    }
}
