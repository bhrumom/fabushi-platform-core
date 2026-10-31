use crate::access::{AccessScope, FileAccessTokenStore};
use crate::actor::ActorId;
use crate::realtime::CallSignal;
use crate::signaling::{SignalingError, SignalingHub};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use thiserror::Error;

pub const MAX_SIGNAL_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum SignalingClientFrame {
    Hello {
        access_token: String,
        actor_id: ActorId,
        device_id: String,
        session_id: String,
    },
    Signal {
        signal: CallSignal,
    },
    Ping {
        nonce: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum SignalingServerFrame {
    Ready { actor_id: ActorId },
    Signal { signal: CallSignal },
    Pong { nonce: String },
    Error { code: String, message: String },
}

#[derive(Debug, Clone)]
pub struct CallSignalingServerConfig {
    pub bind_address: String,
    pub access_registry_path: PathBuf,
    pub max_frame_bytes: usize,
}

impl CallSignalingServerConfig {
    pub fn new(bind_address: impl Into<String>, access_registry_path: impl Into<PathBuf>) -> Self {
        Self {
            bind_address: bind_address.into(),
            access_registry_path: access_registry_path.into(),
            max_frame_bytes: MAX_SIGNAL_FRAME_BYTES,
        }
    }

    pub fn validate(&self) -> Result<(), CallSignalingServerError> {
        if self.bind_address.trim().is_empty() {
            return Err(CallSignalingServerError::InvalidConfig(
                "bind address is empty".into(),
            ));
        }
        if self.access_registry_path.as_os_str().is_empty() {
            return Err(CallSignalingServerError::InvalidConfig(
                "access registry path is empty".into(),
            ));
        }
        if self.max_frame_bytes < 1024 {
            return Err(CallSignalingServerError::InvalidConfig(
                "maximum frame size is too small".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum CallSignalingServerError {
    #[error("call signaling configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("call signaling I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("call signaling JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("call signaling hub lock is poisoned")]
    Poisoned,
    #[error("call signaling frame exceeds {0} bytes")]
    FrameTooLarge(usize),
    #[error("call signaling first frame must be hello")]
    HelloRequired,
    #[error("call signaling access token is invalid")]
    Unauthorized,
    #[error("call signaling actor id is invalid")]
    InvalidActor,
    #[error(transparent)]
    Signaling(#[from] SignalingError),
}

pub struct CallSignalingTcpServer {
    config: CallSignalingServerConfig,
    hub: Arc<Mutex<SignalingHub>>,
}

impl CallSignalingTcpServer {
    pub fn new(config: CallSignalingServerConfig) -> Result<Self, CallSignalingServerError> {
        config.validate()?;
        Ok(Self {
            config,
            hub: Arc::new(Mutex::new(SignalingHub::default())),
        })
    }

    pub fn serve(self) -> Result<(), CallSignalingServerError> {
        let listener = TcpListener::bind(&self.config.bind_address)?;
        let access_store = Arc::new(FileAccessTokenStore::new(
            self.config.access_registry_path.clone(),
        ));
        let max_frame_bytes = self.config.max_frame_bytes;
        for incoming in listener.incoming() {
            let stream = match incoming {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("Fabushi call signaling accept failed: {error}");
                    continue;
                }
            };
            let hub = Arc::clone(&self.hub);
            let access_store = Arc::clone(&access_store);
            let _ = thread::Builder::new()
                .name("fabushi-call-signal-connection".into())
                .spawn(move || {
                    if let Err(error) =
                        handle_connection(stream, hub, access_store, max_frame_bytes)
                    {
                        eprintln!("Fabushi call signaling connection failed: {error}");
                    }
                });
        }
        Ok(())
    }
}

fn handle_connection(
    stream: TcpStream,
    hub: Arc<Mutex<SignalingHub>>,
    access_store: Arc<FileAccessTokenStore>,
    max_frame_bytes: usize,
) -> Result<(), CallSignalingServerError> {
    stream.set_nodelay(true)?;
    let reader_stream = stream.try_clone()?;
    let writer = Arc::new(Mutex::new(BufWriter::new(stream)));
    let mut reader = BufReader::new(reader_stream);

    let hello_line = read_line_limited(&mut reader, max_frame_bytes)?
        .ok_or(CallSignalingServerError::HelloRequired)?;
    let hello: SignalingClientFrame = serde_json::from_slice(&hello_line)?;
    let actor_id = match hello {
        SignalingClientFrame::Hello {
            access_token: provided,
            actor_id,
            device_id,
            session_id,
        } => {
            if !actor_id.is_valid() {
                return Err(CallSignalingServerError::InvalidActor);
            }
            let now_ms = now_millis();
            if access_store
                .authorize(
                    provided.as_bytes(),
                    &actor_id,
                    &device_id,
                    &session_id,
                    AccessScope::Calls,
                    now_ms,
                )
                .is_err()
            {
                write_server_frame(
                    &writer,
                    &SignalingServerFrame::Error {
                        code: "unauthorized".into(),
                        message: "call access token is invalid for this actor/device/session"
                            .into(),
                    },
                )?;
                return Err(CallSignalingServerError::Unauthorized);
            }
            actor_id
        }
        _ => return Err(CallSignalingServerError::HelloRequired),
    };

    let subscription = hub
        .lock()
        .map_err(|_| CallSignalingServerError::Poisoned)?
        .connect(actor_id.clone());

    write_server_frame(
        &writer,
        &SignalingServerFrame::Ready {
            actor_id: actor_id.clone(),
        },
    )?;

    let event_writer = Arc::clone(&writer);
    let event_receiver = subscription.receiver;
    let _ = thread::Builder::new()
        .name("fabushi-call-signal-writer".into())
        .spawn(move || {
            while let Ok(signal) = event_receiver.recv() {
                if write_server_frame(&event_writer, &SignalingServerFrame::Signal { signal })
                    .is_err()
                {
                    break;
                }
            }
        });

    while let Some(line) = read_line_limited(&mut reader, max_frame_bytes)? {
        if line.is_empty() {
            continue;
        }
        let frame: SignalingClientFrame = match serde_json::from_slice(&line) {
            Ok(frame) => frame,
            Err(error) => {
                write_server_frame(
                    &writer,
                    &SignalingServerFrame::Error {
                        code: "invalid_json".into(),
                        message: error.to_string(),
                    },
                )?;
                continue;
            }
        };
        match frame {
            SignalingClientFrame::Signal { signal } => {
                let result = hub
                    .lock()
                    .map_err(|_| CallSignalingServerError::Poisoned)?
                    .route(&actor_id, signal);
                if let Err(error) = result {
                    write_server_frame(
                        &writer,
                        &SignalingServerFrame::Error {
                            code: "signal_rejected".into(),
                            message: error.to_string(),
                        },
                    )?;
                }
            }
            SignalingClientFrame::Ping { nonce } => {
                write_server_frame(&writer, &SignalingServerFrame::Pong { nonce })?;
            }
            SignalingClientFrame::Hello { .. } => {
                write_server_frame(
                    &writer,
                    &SignalingServerFrame::Error {
                        code: "already_authenticated".into(),
                        message: "hello may only be sent once".into(),
                    },
                )?;
            }
        }
    }

    hub.lock()
        .map_err(|_| CallSignalingServerError::Poisoned)?
        .disconnect(&actor_id);
    Ok(())
}

fn write_server_frame(
    writer: &Arc<Mutex<BufWriter<TcpStream>>>,
    frame: &SignalingServerFrame,
) -> Result<(), CallSignalingServerError> {
    let mut writer = writer
        .lock()
        .map_err(|_| CallSignalingServerError::Poisoned)?;
    serde_json::to_writer(&mut *writer, frame)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn read_line_limited<R: BufRead>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> Result<Option<Vec<u8>>, CallSignalingServerError> {
    let mut output = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            if output.is_empty() {
                return Ok(None);
            }
            return Ok(Some(output));
        }
        if let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
            if output.len().saturating_add(position) > max_frame_bytes {
                reader.consume(position + 1);
                return Err(CallSignalingServerError::FrameTooLarge(max_frame_bytes));
            }
            output.extend_from_slice(&buffer[..position]);
            reader.consume(position + 1);
            if output.last() == Some(&b'\r') {
                output.pop();
            }
            return Ok(Some(output));
        }
        if output.len().saturating_add(buffer.len()) > max_frame_bytes {
            let consumed = buffer.len();
            reader.consume(consumed);
            drain_until_newline(reader)?;
            return Err(CallSignalingServerError::FrameTooLarge(max_frame_bytes));
        }
        output.extend_from_slice(buffer);
        let consumed = buffer.len();
        reader.consume(consumed);
    }
}

fn drain_until_newline<R: BufRead>(reader: &mut R) -> Result<(), std::io::Error> {
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(());
        }
        if let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
            reader.consume(position + 1);
            return Ok(());
        }
        let consumed = buffer.len();
        reader.consume(consumed);
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn limited_reader_rejects_oversized_frames_without_unbounded_growth() {
        let payload = format!("{}\nnext\n", "x".repeat(32));
        let mut cursor = Cursor::new(payload.into_bytes());
        let error = read_line_limited(&mut cursor, 16).unwrap_err();
        assert!(matches!(error, CallSignalingServerError::FrameTooLarge(16)));
        let next = read_line_limited(&mut cursor, 16).unwrap().unwrap();
        assert_eq!(next, b"next");
    }

    #[test]
    fn signaling_config_requires_access_registry_path() {
        let config = CallSignalingServerConfig::new("127.0.0.1:9410", "");
        assert!(config.validate().is_err());
    }
}
