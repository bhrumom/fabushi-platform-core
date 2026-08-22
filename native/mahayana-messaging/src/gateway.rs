use crate::access::FileAccessTokenStore;
use crate::server::{
    execute_authenticated_frame, load_shared_messaging_service, now_millis,
    AuthenticatedClientFrame, MessagingServerConfig, MessagingServerError, ServerFrame,
    SharedMessagingService,
};
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;
use tungstenite::protocol::{Message, WebSocket, WebSocketConfig};
use tungstenite::{accept_with_config, Error as WebSocketError};

pub const DEFAULT_GATEWAY_HEARTBEAT_INTERVAL_MS: u64 = 30_000;
pub const DEFAULT_GATEWAY_HEARTBEAT_TIMEOUT_MS: u64 = 90_000;
const WEBSOCKET_PROTOCOL_OVERHEAD_BYTES: usize = 1024;

#[derive(Debug, Clone)]
pub struct MessagingWebSocketGatewayConfig {
    pub server: MessagingServerConfig,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_timeout_ms: u64,
}

impl MessagingWebSocketGatewayConfig {
    pub fn new(server: MessagingServerConfig) -> Self {
        Self {
            server,
            heartbeat_interval_ms: DEFAULT_GATEWAY_HEARTBEAT_INTERVAL_MS,
            heartbeat_timeout_ms: DEFAULT_GATEWAY_HEARTBEAT_TIMEOUT_MS,
        }
    }

    pub fn with_heartbeat(mut self, interval_ms: u64, timeout_ms: u64) -> Self {
        self.heartbeat_interval_ms = interval_ms;
        self.heartbeat_timeout_ms = timeout_ms;
        self
    }

    pub fn validate(&self) -> Result<(), MessagingGatewayError> {
        self.server
            .validate()
            .map_err(MessagingGatewayError::Server)?;
        if self.heartbeat_interval_ms == 0 {
            return Err(MessagingGatewayError::InvalidConfig(
                "heartbeat interval must be greater than zero".into(),
            ));
        }
        if self.heartbeat_timeout_ms < self.heartbeat_interval_ms {
            return Err(MessagingGatewayError::InvalidConfig(
                "heartbeat timeout must be greater than or equal to heartbeat interval".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum MessagingGatewayError {
    #[error("messaging WebSocket gateway configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error(transparent)]
    Server(#[from] MessagingServerError),
    #[error("messaging WebSocket I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("messaging WebSocket JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("messaging WebSocket handshake failed: {0}")]
    Handshake(String),
    #[error("messaging WebSocket transport failed: {0}")]
    WebSocket(#[from] WebSocketError),
    #[error("messaging WebSocket heartbeat timed out")]
    HeartbeatTimeout,
}

pub struct MessagingWebSocketGateway {
    config: MessagingWebSocketGatewayConfig,
    service: SharedMessagingService,
}

impl MessagingWebSocketGateway {
    pub fn load(config: MessagingWebSocketGatewayConfig) -> Result<Self, MessagingGatewayError> {
        config.validate()?;
        let service = load_shared_messaging_service(&config.server)?;
        Ok(Self { config, service })
    }

    pub fn serve(self) -> Result<(), MessagingGatewayError> {
        let listener = TcpListener::bind(&self.config.server.bind_address)?;
        self.serve_listener(listener)
    }

    pub fn serve_listener(self, listener: TcpListener) -> Result<(), MessagingGatewayError> {
        let access_store = Arc::new(FileAccessTokenStore::new(
            self.config.server.access_registry_path.clone(),
        ));
        for incoming in listener.incoming() {
            let stream = match incoming {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("Fabushi WebSocket gateway accept failed: {error}");
                    continue;
                }
            };
            let service = Arc::clone(&self.service);
            let access_store = Arc::clone(&access_store);
            let config = self.config.clone();
            thread::spawn(move || {
                if let Err(error) =
                    handle_websocket_connection(stream, service, access_store, config)
                {
                    eprintln!("Fabushi WebSocket connection failed: {error}");
                }
            });
        }
        Ok(())
    }

    pub fn serve_one(self, listener: TcpListener) -> Result<(), MessagingGatewayError> {
        let (stream, _) = listener.accept()?;
        let access_store = Arc::new(FileAccessTokenStore::new(
            self.config.server.access_registry_path.clone(),
        ));
        handle_websocket_connection(stream, self.service, access_store, self.config)
    }
}

fn handle_websocket_connection(
    stream: TcpStream,
    service: SharedMessagingService,
    access_store: Arc<FileAccessTokenStore>,
    config: MessagingWebSocketGatewayConfig,
) -> Result<(), MessagingGatewayError> {
    stream.set_nodelay(true)?;
    let mut websocket_config = WebSocketConfig::default();
    websocket_config.max_message_size = Some(
        config
            .server
            .max_frame_bytes
            .saturating_add(WEBSOCKET_PROTOCOL_OVERHEAD_BYTES),
    );
    websocket_config.max_frame_size = websocket_config.max_message_size;
    let mut websocket = accept_with_config(stream, Some(websocket_config))
        .map_err(|error| MessagingGatewayError::Handshake(error.to_string()))?;
    websocket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(config.heartbeat_interval_ms)))?;

    let heartbeat_timeout = Duration::from_millis(config.heartbeat_timeout_ms);
    let mut last_peer_activity = Instant::now();
    let mut heartbeat_sequence = 0u64;

    loop {
        match websocket.read() {
            Ok(Message::Text(text)) => {
                last_peer_activity = Instant::now();
                if text.len() > config.server.max_frame_bytes {
                    send_server_frame(
                        &mut websocket,
                        &ServerFrame::Error {
                            code: "frame_too_large".into(),
                            message: format!(
                                "frame exceeds {} bytes",
                                config.server.max_frame_bytes
                            ),
                        },
                    )?;
                    continue;
                }
                let request: AuthenticatedClientFrame = match serde_json::from_str(text.as_str()) {
                    Ok(request) => request,
                    Err(error) => {
                        send_server_frame(
                            &mut websocket,
                            &ServerFrame::Error {
                                code: "invalid_json".into(),
                                message: error.to_string(),
                            },
                        )?;
                        continue;
                    }
                };
                let frame =
                    execute_authenticated_frame(&service, &access_store, request, now_millis())?;
                send_server_frame(&mut websocket, &frame)?;
            }
            Ok(Message::Binary(_)) => {
                last_peer_activity = Instant::now();
                send_server_frame(
                    &mut websocket,
                    &ServerFrame::Error {
                        code: "unsupported_binary".into(),
                        message: "Messaging Protocol v2 application frames must be UTF-8 JSON text"
                            .into(),
                    },
                )?;
            }
            Ok(Message::Ping(_)) => {
                last_peer_activity = Instant::now();
                websocket.flush()?;
            }
            Ok(Message::Pong(_)) => {
                last_peer_activity = Instant::now();
            }
            Ok(Message::Close(_)) => {
                let _ = websocket.flush();
                break;
            }
            Ok(Message::Frame(_)) => {}
            Err(WebSocketError::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                if last_peer_activity.elapsed() >= heartbeat_timeout {
                    return Err(MessagingGatewayError::HeartbeatTimeout);
                }
                heartbeat_sequence = heartbeat_sequence.wrapping_add(1);
                websocket.send(Message::Ping(
                    heartbeat_sequence.to_be_bytes().to_vec().into(),
                ))?;
            }
            Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn send_server_frame(
    websocket: &mut WebSocket<TcpStream>,
    frame: &ServerFrame,
) -> Result<(), MessagingGatewayError> {
    websocket.send(Message::text(serde_json::to_string(frame)?))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_configuration_rejects_zero_and_inverted_timeouts() {
        let server = MessagingServerConfig::new(
            "127.0.0.1:0",
            "/tmp/fabushi-gateway-test.sqlite3",
            "/tmp/fabushi-gateway-access.json",
        );
        assert!(MessagingWebSocketGatewayConfig::new(server.clone())
            .with_heartbeat(0, 1)
            .validate()
            .is_err());
        assert!(MessagingWebSocketGatewayConfig::new(server)
            .with_heartbeat(100, 99)
            .validate()
            .is_err());
    }
}
