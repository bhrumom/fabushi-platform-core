use crate::access::{AccessScope, FileAccessTokenStore};
use crate::blob_store::FileBlobStore;
use crate::protocol::{ClientCommand, ClientEnvelope, ServerEnvelope};
use crate::service::{MessagingService, MessagingServiceError};
use crate::store::{JsonFileStateStore, SqliteStateStore, StoreError};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const MAX_SERVER_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticatedClientFrame {
    pub access_token: String,
    pub envelope: ClientEnvelope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum ServerFrame {
    Events { events: Vec<ServerEnvelope> },
    Error { code: String, message: String },
}

#[derive(Debug, Clone)]
pub struct MessagingServerConfig {
    pub bind_address: String,
    pub database_path: PathBuf,
    pub legacy_snapshot_path: Option<PathBuf>,
    pub access_registry_path: PathBuf,
    pub max_frame_bytes: usize,
}

impl MessagingServerConfig {
    pub fn new(
        bind_address: impl Into<String>,
        database_path: impl Into<PathBuf>,
        access_registry_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            bind_address: bind_address.into(),
            database_path: database_path.into(),
            legacy_snapshot_path: None,
            access_registry_path: access_registry_path.into(),
            max_frame_bytes: MAX_SERVER_FRAME_BYTES,
        }
    }

    pub fn with_legacy_snapshot(mut self, path: impl Into<PathBuf>) -> Self {
        self.legacy_snapshot_path = Some(path.into());
        self
    }

    pub fn validate(&self) -> Result<(), MessagingServerError> {
        if self.bind_address.trim().is_empty() {
            return Err(MessagingServerError::InvalidConfig(
                "bind address is empty".into(),
            ));
        }
        if self.database_path.as_os_str().is_empty() {
            return Err(MessagingServerError::InvalidConfig(
                "database path is empty".into(),
            ));
        }
        if self.access_registry_path.as_os_str().is_empty() {
            return Err(MessagingServerError::InvalidConfig(
                "access registry path is empty".into(),
            ));
        }
        if self.max_frame_bytes < 1024 {
            return Err(MessagingServerError::InvalidConfig(
                "maximum frame size is too small".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum MessagingServerError {
    #[error("messaging server configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("messaging server I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("messaging server JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("messaging store failed: {0}")]
    Store(#[from] StoreError),
    #[error("messaging service failed: {0}")]
    Service(#[from] MessagingServiceError),
    #[error("messaging service lock is poisoned")]
    Poisoned,
}

pub(crate) type SharedMessagingService = Arc<Mutex<MessagingService<SqliteStateStore>>>;

pub struct MessagingTcpServer {
    config: MessagingServerConfig,
    service: SharedMessagingService,
}

impl MessagingTcpServer {
    pub fn load(config: MessagingServerConfig) -> Result<Self, MessagingServerError> {
        config.validate()?;
        let service = load_shared_messaging_service(&config)?;
        Ok(Self { config, service })
    }

    pub fn database_path(&self) -> &Path {
        &self.config.database_path
    }

    pub fn snapshot_path(&self) -> &Path {
        self.database_path()
    }

    pub fn serve(self) -> Result<(), MessagingServerError> {
        let listener = TcpListener::bind(&self.config.bind_address)?;
        let access_store = Arc::new(FileAccessTokenStore::new(
            self.config.access_registry_path.clone(),
        ));
        let max_frame_bytes = self.config.max_frame_bytes;
        for incoming in listener.incoming() {
            let stream = match incoming {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("Fabushi messaging accept failed: {error}");
                    continue;
                }
            };
            let service = Arc::clone(&self.service);
            let access_store = Arc::clone(&access_store);
            thread::spawn(move || {
                if let Err(error) =
                    handle_connection(stream, service, access_store, max_frame_bytes)
                {
                    eprintln!("Fabushi messaging connection failed: {error}");
                }
            });
        }
        Ok(())
    }
}

pub(crate) fn load_shared_messaging_service(
    config: &MessagingServerConfig,
) -> Result<SharedMessagingService, MessagingServerError> {
    let mut store = SqliteStateStore::new(config.database_path.clone());
    if let Some(legacy_path) = config.legacy_snapshot_path.as_ref() {
        let legacy = JsonFileStateStore::new(legacy_path);
        store.import_json_if_empty(&legacy)?;
    }
    let blob_root = config
        .database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("blobs");
    let service = MessagingService::load_with_blob_store(store, FileBlobStore::new(blob_root))?;
    Ok(Arc::new(Mutex::new(service)))
}

pub(crate) fn execute_authenticated_frame(
    service: &SharedMessagingService,
    access_store: &FileAccessTokenStore,
    request: AuthenticatedClientFrame,
    now_ms: i64,
) -> Result<ServerFrame, MessagingServerError> {
    let context = &request.envelope.context;
    if let Err(error) = access_store.authorize(
        request.access_token.as_bytes(),
        &context.actor_id,
        &context.device_id,
        &context.session_id,
        required_scope(&request.envelope.command),
        now_ms,
    ) {
        return Ok(ServerFrame::Error {
            code: "unauthorized".into(),
            message: format!("messaging access denied: {error}"),
        });
    }
    let mut service = service.lock().map_err(|_| MessagingServerError::Poisoned)?;
    let frame = match service.handle(request.envelope, now_ms) {
        Ok(events) => ServerFrame::Events { events },
        Err(error) => ServerFrame::Error {
            code: "messaging_error".into(),
            message: error.to_string(),
        },
    };
    Ok(frame)
}

fn handle_connection(
    stream: TcpStream,
    service: SharedMessagingService,
    access_store: Arc<FileAccessTokenStore>,
    max_frame_bytes: usize,
) -> Result<(), MessagingServerError> {
    stream.set_nodelay(true)?;
    let reader_stream = stream.try_clone()?;
    let mut reader = BufReader::new(reader_stream);
    let mut writer = BufWriter::new(stream);
    let mut line = Vec::new();

    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        if line.len() > max_frame_bytes {
            write_frame(
                &mut writer,
                &ServerFrame::Error {
                    code: "frame_too_large".into(),
                    message: format!("frame exceeds {max_frame_bytes} bytes"),
                },
            )?;
            break;
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        if line.is_empty() {
            continue;
        }
        let request: AuthenticatedClientFrame = match serde_json::from_slice(&line) {
            Ok(request) => request,
            Err(error) => {
                write_frame(
                    &mut writer,
                    &ServerFrame::Error {
                        code: "invalid_json".into(),
                        message: error.to_string(),
                    },
                )?;
                continue;
            }
        };
        let frame = execute_authenticated_frame(&service, &access_store, request, now_millis())?;
        write_frame(&mut writer, &frame)?;
    }
    Ok(())
}

fn write_frame(
    writer: &mut BufWriter<TcpStream>,
    frame: &ServerFrame,
) -> Result<(), MessagingServerError> {
    serde_json::to_writer(&mut *writer, frame)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub(crate) fn required_scope(command: &ClientCommand) -> AccessScope {
    match command {
        ClientCommand::BeginBlobUpload { .. }
        | ClientCommand::AppendBlobChunk { .. }
        | ClientCommand::FinishBlobUpload { .. }
        | ClientCommand::DeleteBlob { .. } => AccessScope::BlobsWrite,
        ClientCommand::CreateInvoice { .. }
        | ClientCommand::CheckoutInvoice { .. }
        | ClientCommand::RefundOrder { .. }
        | ClientCommand::WalletStatus => AccessScope::Payments,
        ClientCommand::InstallMiniApp { .. }
        | ClientCommand::GrantMiniApp { .. }
        | ClientCommand::OpenMiniApp { .. }
        | ClientCommand::MiniAppCall { .. } => AccessScope::MiniApps,
        _ => AccessScope::Messaging,
    }
}

pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privileged_commands_require_narrow_scopes() {
        let blob = ClientCommand::DeleteBlob {
            blob_id: crate::blob_store::BlobId::new("blob-1").unwrap(),
        };
        assert_eq!(required_scope(&blob), AccessScope::BlobsWrite);
        let mini_app = ClientCommand::MiniAppCall {
            session_id: "session".into(),
            request_id: "request".into(),
            request: crate::miniapp::MiniAppRequest::Close,
        };
        assert_eq!(required_scope(&mini_app), AccessScope::MiniApps);
    }

    #[test]
    fn production_config_requires_database_and_access_registry_paths() {
        let missing_database = MessagingServerConfig::new("127.0.0.1:9400", "", "/tmp/access.json");
        assert!(missing_database.validate().is_err());

        let missing_access =
            MessagingServerConfig::new("127.0.0.1:9400", "/tmp/messages.sqlite3", "");
        assert!(missing_access.validate().is_err());
    }
}
