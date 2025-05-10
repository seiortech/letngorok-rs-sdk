use crate::config::{SDKConfig, TunnelConfig};
use crate::error::TunnelError;
use crate::types::{
    TunnelMessage, TunnelMessageType, TunnelStatus, HEADER_CONTENT_TYPE, HEADER_HOST,
    HEADER_LOCAL_URL, HEADER_PROD_URL, HEADER_X_FORWARDED_HOST, HEADER_X_STATUS_CODE,
};
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::StatusCode;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, Notify};
use tokio::time::timeout;

pub struct TunnelConn {
    local_url: Arc<Mutex<Option<String>>>,
    prod_url: Arc<Mutex<Option<String>>>,
    tunnel_id: Arc<Mutex<Option<String>>>,

    config: Arc<TunnelConfig>,
    sdk_config: Arc<SDKConfig>,
    http_client: reqwest::Client,

    writer: Arc<Mutex<Option<WriteHalf<TcpStream>>>>,
    status: Arc<Mutex<TunnelStatus>>,
    shutdown_notify: Arc<Notify>,
    handler_task_join_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl TunnelConn {
    pub fn new(
        tunnel_specific_config: TunnelConfig,
        sdk_config: Arc<SDKConfig>,
        port: String,
    ) -> Result<Self, TunnelError> {
        let mut cfg = tunnel_specific_config;
        cfg.local_port = port;
        let arc_cfg = Arc::new(cfg);

        let http_client = reqwest::Client::builder()
            .timeout(arc_cfg.request_timeout)
            .build()?;

        Ok(TunnelConn {
            local_url: Arc::new(Mutex::new(None)),
            prod_url: Arc::new(Mutex::new(None)),
            tunnel_id: Arc::new(Mutex::new(None)),
            config: arc_cfg,
            sdk_config,
            http_client,
            writer: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(TunnelStatus::Disconnected)),
            shutdown_notify: Arc::new(Notify::new()),
            handler_task_join_handle: Mutex::new(None),
        })
    }

    async fn set_status(&self, new_status: TunnelStatus) {
        *self.status.lock().await = new_status;
    }

    async fn connect_and_authenticate(
        &self,
    ) -> Result<BufReader<ReadHalf<TcpStream>>, TunnelError> {
        self.set_status(TunnelStatus::Connecting).await;
        self.sdk_config.trigger_on_auth();

        let stream = match timeout(
            self.config.auth_timeout,
            TcpStream::connect(&self.sdk_config.tunnel_server),
        )
        .await
        {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                let err = TunnelError::DialError(e.to_string());
                self.set_status(TunnelStatus::Error).await;
                self.sdk_config.trigger_on_error(&err);
                return Err(err);
            }
            Err(_) => {
                let err = TunnelError::OperationTimeout(format!(
                    "connecting to {}",
                    self.sdk_config.tunnel_server
                ));
                self.set_status(TunnelStatus::Error).await;
                self.sdk_config.trigger_on_error(&err);
                return Err(err);
            }
        };

        let (reader_half, mut writer_stream_half) = tokio::io::split(stream);

        self.set_status(TunnelStatus::Authenticating).await;
        let auth_message = TunnelMessage {
            message_type: TunnelMessageType::TunnelAuthRequest,
            body: Some(self.sdk_config.auth_token.clone()),
            id: None,
            method: None,
            path: None,
            headers: None,
        };

        let auth_payload = serde_json::to_string(&auth_message)? + "\n";
        if let Err(e) = writer_stream_half.write_all(auth_payload.as_bytes()).await {
            self.set_status(TunnelStatus::Error).await;
            let err = TunnelError::Io(e);
            self.sdk_config.trigger_on_error(&err);
            return Err(err);
        }

        *self.writer.lock().await = Some(writer_stream_half);

        let mut buf_reader = BufReader::new(reader_half);
        let mut response_line = String::new();

        match timeout(
            self.config.auth_timeout,
            buf_reader.read_line(&mut response_line),
        )
        .await
        {
            Ok(Ok(0)) => {
                self.set_status(TunnelStatus::Error).await;
                let err = TunnelError::ConnectionClosed;
                self.sdk_config.trigger_on_error(&err);
                self.close_writer_half().await;
                return Err(err);
            }
            Ok(Ok(_)) => { /* do nothing, bytes read at below */ }
            Ok(Err(e)) => {
                self.set_status(TunnelStatus::Error).await;
                let err = TunnelError::Io(e);
                self.sdk_config.trigger_on_error(&err);
                self.close_writer_half().await;
                return Err(err);
            }
            Err(_) => {
                self.set_status(TunnelStatus::Error).await;
                let err = TunnelError::OperationTimeout("authentication response".into());
                self.sdk_config.trigger_on_error(&err);
                self.close_writer_half().await;
                return Err(err);
            }
        }

        let server_msg: TunnelMessage =
            serde_json::from_str(response_line.trim()).map_err(|e| {
                let err = TunnelError::Json(e);
                self.sdk_config.trigger_on_error(&err);
                err
            })?;

        if server_msg.message_type == TunnelMessageType::TunnelAuthFailure {
            self.set_status(TunnelStatus::Error).await;
            let err_msg = server_msg
                .body
                .unwrap_or_else(|| "Unknown authentication error".to_string());
            let err = TunnelError::AuthFailure(err_msg);
            self.sdk_config.trigger_on_error(&err);
            self.close_writer_half().await;
            return Err(err);
        }

        self.set_status(TunnelStatus::Establishing).await;

        if server_msg.message_type != TunnelMessageType::TunnelCreated {
            self.set_status(TunnelStatus::Error).await;
            let err = TunnelError::TunnelNotCreatedAfterAuth;
            self.sdk_config.trigger_on_error(&err);
            self.close_writer_half().await;
            return Err(err);
        }

        let headers = server_msg
            .headers
            .ok_or_else(|| TunnelError::MissingHeader("headers for TunnelCreated".to_string()))?;

        let local_url_val = headers
            .get(HEADER_LOCAL_URL)
            .cloned()
            .ok_or_else(|| TunnelError::MissingHeader(HEADER_LOCAL_URL.to_string()))?;
        let prod_url_val = headers
            .get(HEADER_PROD_URL)
            .cloned()
            .ok_or_else(|| TunnelError::MissingHeader(HEADER_PROD_URL.to_string()))?;
        let tunnel_id_val = server_msg
            .id
            .ok_or_else(|| TunnelError::MissingHeader("tunnel_id (message.id)".to_string()))?;

        *self.local_url.lock().await = Some(local_url_val.clone());
        *self.prod_url.lock().await = Some(prod_url_val.clone());
        *self.tunnel_id.lock().await = Some(tunnel_id_val.clone());

        self.set_status(TunnelStatus::Connected).await;
        self.sdk_config.trigger_on_connected(
            &self.config.local_port,
            &local_url_val,
            &prod_url_val,
            &tunnel_id_val,
        );

        Ok(buf_reader)
    }

    async fn close_writer_half(&self) {
        if let Some(mut writer) = self.writer.lock().await.take() {
            let _ = writer.shutdown().await;
        }
    }

    pub async fn start_handling_requests(self: Arc<Self>) {
        let mut task_handle_guard = self.handler_task_join_handle.lock().await;
        if task_handle_guard.is_some() {
            tracing::warn!("TunnelConn handler task already started.");
            return;
        }

        let conn_clone_for_task = Arc::clone(&self);
        let handle = tokio::spawn(async move {
            match conn_clone_for_task.connect_and_authenticate().await {
                Ok(buf_reader) => {
                    conn_clone_for_task
                        .handle_incoming_messages(buf_reader)
                        .await;
                }
                Err(e) => {
                    tracing::debug!(
                        "connect_and_authenticate failed: {:?}, handler task will exit.",
                        e
                    );
                    if *conn_clone_for_task.status.lock().await != TunnelStatus::Disconnected {
                        conn_clone_for_task
                            .set_status(TunnelStatus::Disconnected)
                            .await;
                        conn_clone_for_task.sdk_config.trigger_on_disconnected();
                    }
                }
            }
        });
        *task_handle_guard = Some(handle);
    }

    async fn handle_incoming_messages(self: Arc<Self>, mut reader: BufReader<ReadHalf<TcpStream>>) {
        let mut line_buffer = String::new();
        loop {
            tokio::select! {
                _ = self.shutdown_notify.notified() => {
                    tracing::debug!("Shutdown signal received, exiting message handling loop.");
                    break;
                }
                read_result = reader.read_line(&mut line_buffer) => {
                    match read_result {
                        Ok(0) => {
                            tracing::info!("Connection closed by server (EOF).");
                            self.sdk_config.trigger_on_error(&TunnelError::ConnectionClosed);
                            break;
                        }
                        Ok(_) => {
                            let msg_str = line_buffer.trim_end();
                            if msg_str.is_empty() {
                                line_buffer.clear();
                                continue;
                            }
                            match serde_json::from_str::<TunnelMessage>(msg_str) {
                                Ok(msg) => {
                                    if msg.message_type == TunnelMessageType::TunnelRequest {
                                        let self_clone_for_request = Arc::clone(&self);
                                        tokio::spawn(async move {
                                            self_clone_for_request.handle_local_service_request(msg).await;
                                        });
                                    } else {
                                        let err = TunnelError::UnexpectedMessageType(msg.message_type);
                                        tracing::warn!("{:?}", err);
                                        self.sdk_config.trigger_on_error(&err);
                                    }
                                }
                                Err(e) => {
                                    let err = TunnelError::Json(e);
                                    tracing::error!("Failed to decode message: {:?}, content: '{}'", err, msg_str);
                                    self.sdk_config.trigger_on_error(&err);
                                }
                            }
                            line_buffer.clear();
                        }
                        Err(e) => {
                            tracing::error!("Error reading from tunnel connection: {}", e);
                            self.sdk_config.trigger_on_error(&TunnelError::Io(e));
                            break;
                        }
                    }
                }
            }
        }

        self.set_status(TunnelStatus::Disconnected).await;
        self.sdk_config.trigger_on_disconnected();
        self.close_writer_half().await;
    }

    async fn handle_local_service_request(self: Arc<Self>, msg: TunnelMessage) {
        self.sdk_config.trigger_on_request(&msg);

        let request_id_str = msg.id.as_deref().unwrap_or_default();

        let target_path = msg.path.as_deref().unwrap_or("/");
        let target_url = format!("http://localhost:{}{}", self.config.local_port, target_path);

        let http_method_str = msg.method.as_deref().unwrap_or("GET");
        let http_method = match reqwest::Method::from_str(http_method_str) {
            Ok(m) => m,
            Err(_) => {
                let err_text = format!("Invalid HTTP method: {}", http_method_str);
                tracing::error!("{}", err_text);
                self.sdk_config
                    .trigger_on_error(&TunnelError::InternalError(err_text.clone()));
                self.send_error_response_to_tunnel(
                    request_id_str,
                    StatusCode::BAD_REQUEST,
                    &err_text,
                )
                .await;
                return;
            }
        };

        let mut request_builder = self.http_client.request(http_method, &target_url);

        if let Some(body_str) = msg.body.as_ref() {
            request_builder = request_builder.body(body_str.clone());
        }

        let mut effective_host_for_local_request = format!("localhost:{}", self.config.local_port);

        if let Some(headers_from_tunnel) = msg.headers.as_ref() {
            for (key, value) in headers_from_tunnel {
                if key.eq_ignore_ascii_case(HEADER_HOST) {
                    // use X-Forwarded-Host or default to local target.
                    continue;
                }
                if key.eq_ignore_ascii_case(HEADER_X_FORWARDED_HOST) {
                    effective_host_for_local_request = value.clone();
                }

                match HeaderName::from_str(key) {
                    Ok(h_name) => match HeaderValue::from_str(value) {
                        Ok(h_val) => {
                            request_builder = request_builder.header(h_name, h_val);
                        }
                        Err(e) => tracing::warn!(
                            "Invalid header value for {}: {} ({}). Skipping.",
                            key,
                            value,
                            e
                        ),
                    },
                    Err(e) => tracing::warn!("Invalid header name {}: ({}). Skipping.", key, e),
                }
            }
        }
        request_builder =
            request_builder.header(reqwest::header::HOST, effective_host_for_local_request);

        match timeout(self.config.request_timeout, request_builder.send()).await {
            Ok(Ok(local_service_response)) => {
                let status = local_service_response.status();
                let response_headers_map = local_service_response.headers().clone();

                match local_service_response.bytes().await {
                    Ok(body_bytes) => {
                        self.sdk_config.trigger_on_sending_response(
                            &msg,
                            status,
                            &response_headers_map,
                            &body_bytes,
                        );

                        let mut tunnel_response_headers = HashMap::new();
                        for (key, value) in response_headers_map.iter() {
                            if let Ok(val_str) = value.to_str() {
                                tunnel_response_headers
                                    .insert(key.as_str().to_string(), val_str.to_string());
                            }
                        }
                        tunnel_response_headers.insert(
                            HEADER_X_STATUS_CODE.to_string(),
                            status.as_u16().to_string(),
                        );

                        let response_msg_to_tunnel = TunnelMessage {
                            message_type: TunnelMessageType::TunnelResponse,
                            id: msg.id.clone(),
                            headers: Some(tunnel_response_headers),
                            body: Some(String::from_utf8_lossy(&body_bytes).into_owned()),
                            method: None,
                            path: None,
                        };
                        self.send_message_to_tunnel(response_msg_to_tunnel).await;
                    }
                    Err(e) => {
                        let err_text = format!("Failed to read local response body: {}", e);
                        tracing::error!("{}", err_text);
                        self.sdk_config
                            .trigger_on_error(&TunnelError::HttpClient(e));
                        self.send_error_response_to_tunnel(
                            request_id_str,
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Failed to read local response body",
                        )
                        .await;
                    }
                }
            }
            Ok(Err(e)) => {
                let (status_code, err_text) = if e.is_timeout() {
                    (
                        StatusCode::GATEWAY_TIMEOUT,
                        format!("Timeout connecting to local service {}: {}", target_url, e),
                    )
                } else {
                    (
                        StatusCode::BAD_GATEWAY,
                        format!("Error connecting to local service {}: {}", target_url, e),
                    )
                };
                tracing::error!("{}", err_text);
                self.sdk_config
                    .trigger_on_error(&TunnelError::HttpClient(e));
                self.send_error_response_to_tunnel(request_id_str, status_code, &err_text)
                    .await;
            }
            Err(_timeout_err) => {
                let err_text = format!("Request to local service {} timed out", target_url);
                tracing::error!("{}", err_text);
                self.sdk_config
                    .trigger_on_error(&TunnelError::OperationTimeout(
                        "local service request".into(),
                    ));
                self.send_error_response_to_tunnel(
                    request_id_str,
                    StatusCode::GATEWAY_TIMEOUT,
                    "Local service timed out",
                )
                .await;
            }
        }
    }

    async fn send_message_to_tunnel(&self, msg: TunnelMessage) {
        let payload = match serde_json::to_string(&msg) {
            Ok(p) => p + "\n", // Line-delimit messages
            Err(e) => {
                let err = TunnelError::Json(e);
                tracing::error!("Failed to serialize message for tunnel: {:?}", err);
                self.sdk_config.trigger_on_error(&err);
                return;
            }
        };

        let mut writer_guard = self.writer.lock().await;
        if let Some(writer) = writer_guard.as_mut() {
            if let Err(e) = writer.write_all(payload.as_bytes()).await {
                let err = TunnelError::Io(e);
                tracing::error!("Failed to send message to tunnel: {:?}", err);
                self.sdk_config.trigger_on_error(&err);
            }
        } else {
            tracing::warn!("Attempted to send message, but tunnel writer is not available.");
            self.sdk_config.trigger_on_error(&TunnelError::NotConnected);
        }
    }

    async fn send_error_response_to_tunnel(
        &self,
        request_id: &str,
        status_code: StatusCode,
        message: &str,
    ) {
        let response_msg = TunnelMessage {
            message_type: TunnelMessageType::TunnelResponse,
            id: Some(request_id.to_string()),
            headers: Some(HashMap::from([
                (
                    HEADER_X_STATUS_CODE.to_string(),
                    status_code.as_u16().to_string(),
                ),
                (
                    HEADER_CONTENT_TYPE.to_string(),
                    "text/plain; charset=utf-8".to_string(),
                ),
            ])),
            body: Some(format!(
                "{} {}: {}",
                status_code.as_u16(),
                status_code.canonical_reason().unwrap_or("Error"),
                message
            )),
            method: None,
            path: None,
        };
        self.send_message_to_tunnel(response_msg).await;
    }

    pub async fn stop(&self) -> Result<(), TunnelError> {
        tracing::debug!("Stopping TunnelConn for port {}...", self.config.local_port);
        self.shutdown_notify.notify_waiters(); // Signal message handling loop to stop

        let mut task_handle_guard = self.handler_task_join_handle.lock().await;
        if let Some(handle) = task_handle_guard.take() {
            tracing::debug!("Waiting for TunnelConn handler task to finish...");
            if let Err(e) = handle.await {
                let err = TunnelError::TaskJoinError(e);
                tracing::error!("TunnelConn handler task failed to join cleanly: {:?}", err);
                self.sdk_config.trigger_on_error(&err);
            } else {
                tracing::debug!("TunnelConn handler task finished.");
            }
        }

        self.close_writer_half().await;

        if *self.status.lock().await != TunnelStatus::Disconnected {
            self.set_status(TunnelStatus::Disconnected).await;
            self.sdk_config.trigger_on_disconnected();
        }
        tracing::info!("TunnelConn for port {} stopped.", self.config.local_port);
        Ok(())
    }

    pub async fn get_status(&self) -> TunnelStatus {
        self.status.lock().await.clone()
    }
}
