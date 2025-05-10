use crate::config::{SDKConfig, TunnelConfig};
use crate::connection::TunnelConn;
use crate::error::TunnelError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct TunnelClient {
    connections: Arc<Mutex<HashMap<String, Arc<TunnelConn>>>>,
    sdk_config: Arc<SDKConfig>,
}

impl TunnelClient {
    pub fn new(mut user_sdk_config: Option<SDKConfig>, token: String) -> Result<Self, TunnelError> {
        if token.is_empty() {
            return Err(TunnelError::NoAuthTokenProvided);
        }

        let mut effective_config = SDKConfig::default();
        effective_config.auth_token = token;

        if let Some(u_conf) = user_sdk_config.take() {
            effective_config.tunnel_server = u_conf.tunnel_server;

            if u_conf.on_auth.is_some() {
                effective_config.on_auth = u_conf.on_auth;
            }
            if u_conf.on_connected.is_some() {
                effective_config.on_connected = u_conf.on_connected;
            }
            if u_conf.on_disconnected.is_some() {
                effective_config.on_disconnected = u_conf.on_disconnected;
            }
            if u_conf.on_error.is_some() {
                effective_config.on_error = u_conf.on_error;
            }
            if u_conf.on_request.is_some() {
                effective_config.on_request = u_conf.on_request;
            }
            if u_conf.on_sending_response.is_some() {
                effective_config.on_sending_response = u_conf.on_sending_response;
            }
        }

        Ok(TunnelClient {
            connections: Arc::new(Mutex::new(HashMap::new())),
            sdk_config: Arc::new(effective_config),
        })
    }

    pub async fn start(
        &self,
        port: String,
        tunnel_specific_config: Option<TunnelConfig>,
    ) -> Result<(), TunnelError> {
        if port.parse::<u16>().is_err() || port.is_empty() {
            return Err(TunnelError::InvalidLocalPort(port));
        }

        let mut conns_guard = self.connections.lock().await;
        if conns_guard.contains_key(&port) {
            // Optionally, check if existing conn is active. For now, simple duplicate check.
            return Err(TunnelError::DuplicatePort(port.clone()));
        }

        let tunnel_cfg = tunnel_specific_config.unwrap_or_default();

        let conn = Arc::new(TunnelConn::new(
            tunnel_cfg,
            Arc::clone(&self.sdk_config),
            port.clone(),
        )?);

        Arc::clone(&conn).start_handling_requests().await;

        conns_guard.insert(port, conn);
        Ok(())
    }

    pub async fn stop(&self, port: &str) -> Result<(), TunnelError> {
        let conn_to_stop = {
            let mut conns_guard = self.connections.lock().await;
            conns_guard.remove(port)
        };

        if let Some(conn) = conn_to_stop {
            conn.stop().await?;
        } else {
            tracing::warn!("No active tunnel found for port {} to stop.", port);
        }
        Ok(())
    }

    pub async fn stop_all(&self) -> Result<(), TunnelError> {
        let mut conns_to_stop = Vec::new();
        {
            let mut conns_guard = self.connections.lock().await;
            for (port, conn) in conns_guard.drain() {
                tracing::info!("Preparing to stop tunnel for port {}...", port);
                conns_to_stop.push(conn);
            }
        }

        let mut errors = Vec::new();
        for conn in conns_to_stop {
            if let Err(e) = conn.stop().await {
                errors.push(e);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            let first_error = errors.remove(0);
            tracing::error!(
                "Encountered {} errors while stopping all tunnels. First error: {:?}",
                errors.len() + 1,
                first_error
            );
            Err(first_error)
        }
    }
}
