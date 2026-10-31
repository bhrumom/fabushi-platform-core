//! Native MTProto network transport.
//!
//! This crate owns sockets and connection retry policy. Wire framing,
//! cryptography, and authorization state remain in `telegram-protocol` so the
//! same protocol implementation can be reused by native and Web transports.

use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fabushi_telegram_protocol::{
    decrypt_message, encrypt_message, AuthKey, AuthKeyHandshake, CryptoDirection, CryptoError,
    DcDirectory, DcError, DcPurpose, DhGenAction, EncryptedEnvelope, HandshakeError,
    MessageIdError, MessageIdGuard, PlainMessage, PlaintextEnvelope, SequenceError,
    SessionSequence, TransportError, TransportFrameCodec, TransportMode,
};
use thiserror::Error;
use zeroize::Zeroizing;

const DEFAULT_READ_CHUNK_BYTES: usize = 16 * 1024;
const DEFAULT_MAX_DH_RETRIES: usize = 3;
const DEFAULT_MAX_SERVICE_MESSAGES: usize = 8;
const PING_CONSTRUCTOR: u32 = 0x7abe_77ec;
const PONG_CONSTRUCTOR: u32 = 0x3477_73c5;
const NEW_SESSION_CREATED_CONSTRUCTOR: u32 = 0x9ec2_0908;
const MSG_DETAILED_INFO_CONSTRUCTOR: u32 = 0x276d_3ec6;
const MSG_NEW_DETAILED_INFO_CONSTRUCTOR: u32 = 0x809d_b6df;
const MSGS_STATE_INFO_CONSTRUCTOR: u32 = 0x04de_b57d;
const MSGS_ALL_INFO_CONSTRUCTOR: u32 = 0x8cc0_d131;
const BAD_SERVER_SALT_CONSTRUCTOR: u32 = 0xedab_447b;
const BAD_MSG_NOTIFICATION_CONSTRUCTOR: u32 = 0xa7ef_f811;
const MSGS_ACK_CONSTRUCTOR: u32 = 0x62d6_b459;
const RPC_RESULT_CONSTRUCTOR: u32 = 0xf35c_6d01;
const FUTURE_SALTS_CONSTRUCTOR: u32 = 0xae50_0895;
const DEFAULT_MAX_RPC_MESSAGES: usize = 64;

#[derive(Debug, Clone, Copy)]
pub struct NetworkConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
    pub transport_mode: TransportMode,
    pub prefer_ipv6: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(15),
            write_timeout: Duration::from_secs(15),
            transport_mode: TransportMode::Abridged,
            prefer_ipv6: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedEndpoint {
    pub dc_id: i32,
    pub address: SocketAddr,
}

pub struct EstablishedSession {
    pub endpoint: ConnectedEndpoint,
    pub transport: TcpTransport,
    pub auth_key: AuthKey,
    pub server_salt: i64,
    pub server_time: i32,
    message_ids: MessageIdGuard,
    sequence: SessionSequence,
    session_id: i64,
    clock_offset_seconds: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingResult {
    pub ping_id: i64,
    pub response_message_id: i64,
    pub server_salt: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcResult {
    pub request_message_id: i64,
    pub response_message_id: i64,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("Telegram data center routing failed: {0}")]
    Directory(#[from] DcError),
    #[error("all Telegram data center connection attempts failed: {attempts:?}")]
    ConnectFailed { attempts: Vec<String> },
    #[error("Telegram TCP transport I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("Telegram transport frame failed: {0}")]
    Transport(#[from] TransportError),
    #[error("Telegram auth-key handshake failed: {0}")]
    Handshake(#[from] HandshakeError),
    #[error("Telegram message id failed validation: {0}")]
    MessageId(#[from] MessageIdError),
    #[error("Telegram session sequence failed: {0}")]
    Sequence(#[from] SequenceError),
    #[error("Telegram encrypted message failed: {0}")]
    Crypto(#[from] CryptoError),
    #[error("system clock is earlier than the Unix epoch")]
    InvalidSystemClock,
    #[error("Telegram closed the TCP connection")]
    ConnectionClosed,
    #[error("Telegram returned too many dh_gen_retry responses")]
    DhRetryLimit,
    #[error("operating system secure randomness is unavailable")]
    RandomnessUnavailable,
    #[error("Telegram service message is malformed")]
    InvalidServiceMessage,
    #[error("Telegram pong does not match the sent ping")]
    PingMismatch,
    #[error("Telegram did not return a pong before the service-message limit")]
    PongTimeout,
    #[error("Telegram rejected encrypted message {message_id} with error code {code}")]
    BadServerMessage { message_id: i64, code: i32 },
    #[error("Telegram requested replay of message {request_message_id} before new-session first message {first_message_id} more than once")]
    NewSessionReplayLimit {
        request_message_id: i64,
        first_message_id: i64,
    },
    #[error("Telegram requested state-driven replay of message {request_message_id} more than once")]
    StateReconciliationLimit { request_message_id: i64 },
    #[error("Telegram did not return an RPC result before the message limit")]
    RpcTimeout,
}

pub struct TcpTransport {
    stream: TcpStream,
    codec: TransportFrameCodec,
    read_buffer: Vec<u8>,
    initialized: bool,
}

impl TcpTransport {
    pub fn connect(
        address: SocketAddr,
        mode: TransportMode,
        config: &NetworkConfig,
    ) -> Result<Self, NetworkError> {
        let stream = TcpStream::connect_timeout(&address, config.connect_timeout)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(config.read_timeout))?;
        stream.set_write_timeout(Some(config.write_timeout))?;
        Ok(Self::from_stream(stream, mode))
    }

    pub fn from_stream(stream: TcpStream, mode: TransportMode) -> Self {
        Self {
            stream,
            codec: TransportFrameCodec::new(mode),
            read_buffer: Vec::with_capacity(DEFAULT_READ_CHUNK_BYTES),
            initialized: false,
        }
    }

    pub fn send_payload(&mut self, payload: &[u8]) -> Result<(), NetworkError> {
        if !self.initialized {
            self.stream.write_all(self.codec.initial_header())?;
            self.initialized = true;
        }
        let frame = self.codec.encode(payload)?;
        self.stream.write_all(&frame)?;
        self.stream.flush()?;
        Ok(())
    }

    pub fn receive_payload(&mut self) -> Result<Vec<u8>, NetworkError> {
        loop {
            if let Some(frame) = self.codec.decode(&self.read_buffer)? {
                self.read_buffer.drain(..frame.consumed_bytes);
                if frame.quick_ack_token.is_some() {
                    continue;
                }
                return Ok(frame.payload);
            }

            let mut chunk = [0_u8; DEFAULT_READ_CHUNK_BYTES];
            let count = self.stream.read(&mut chunk)?;
            if count == 0 {
                return Err(NetworkError::ConnectionClosed);
            }
            self.read_buffer.extend_from_slice(&chunk[..count]);
        }
    }

    pub fn exchange_plaintext(
        &mut self,
        request: &PlaintextEnvelope,
    ) -> Result<PlaintextEnvelope, NetworkError> {
        self.send_payload(&request.encode()?)?;
        PlaintextEnvelope::decode(&self.receive_payload()?).map_err(NetworkError::from)
    }

    pub fn shutdown(&self) -> Result<(), NetworkError> {
        self.stream.shutdown(Shutdown::Both)?;
        Ok(())
    }
}

impl EstablishedSession {
    pub fn invoke_raw(&mut self, body: &[u8]) -> Result<RpcResult, NetworkError> {
        if body.is_empty() || !body.len().is_multiple_of(4) {
            return Err(NetworkError::InvalidServiceMessage);
        }
        for attempt in 0..2 {
            let request_message_id = self.send_encrypted_message(body, true)?;
            let mut retry_with_new_salt = false;
            let mut retry_bad_message: Option<(i64, i32)> = None;
            let mut retry_new_session: Option<i64> = None;
            let mut retry_state = false;
            let mut state_query_message_id = None;
            for message_index in 0..(DEFAULT_MAX_RPC_MESSAGES + DEFAULT_MAX_SERVICE_MESSAGES) {
                if message_index == DEFAULT_MAX_RPC_MESSAGES {
                    let state_query =
                        fabushi_telegram_protocol::build_msgs_state_req(&[request_message_id])
                            .map_err(|_| NetworkError::InvalidServiceMessage)?;
                    state_query_message_id =
                        Some(self.send_encrypted_message(&state_query, false)?);
                }
                let payload = self.transport.receive_payload()?;
                let envelope = EncryptedEnvelope::from_bytes(&payload)?;
                let response = decrypt_message(
                    &self.auth_key,
                    CryptoDirection::ServerToClient,
                    &envelope,
                    self.session_id,
                )?;
                self.validate_incoming_message_id(response.message_id, &response.body)?;
                let mut acknowledgements = Vec::new();
                if response.sequence_number.rem_euclid(2) == 1 {
                    acknowledgements.push(response.message_id);
                }
                let outcome = self.scan_rpc_body(
                    &response.body,
                    request_message_id,
                    state_query_message_id,
                    response.message_id,
                    &mut acknowledgements,
                )?;
                if !acknowledgements.is_empty() {
                    self.send_acknowledgements(&acknowledgements)?;
                }
                match outcome {
                    RpcOutcome::Result(result) => return Ok(result),
                    RpcOutcome::NewSalt => {
                        retry_with_new_salt = true;
                        break;
                    }
                    RpcOutcome::RetryBadMessage { message_id, code } => {
                        retry_bad_message = Some((message_id, code));
                        break;
                    }
                    RpcOutcome::RetryNewSession { first_message_id } => {
                        retry_new_session = Some(first_message_id);
                        break;
                    }
                    RpcOutcome::RetryState => {
                        retry_state = true;
                        break;
                    }
                    RpcOutcome::Continue => {}
                }
            }
            if let Some(first_message_id) = retry_new_session {
                if attempt == 0 {
                    continue;
                }
                return Err(NetworkError::NewSessionReplayLimit {
                    request_message_id,
                    first_message_id,
                });
            }
            if let Some((message_id, code)) = retry_bad_message {
                if attempt == 0 {
                    continue;
                }
                return Err(NetworkError::BadServerMessage { message_id, code });
            }
            if retry_state {
                if attempt == 0 {
                    continue;
                }
                return Err(NetworkError::StateReconciliationLimit { request_message_id });
            }
            if !(retry_with_new_salt && attempt == 0) {
                return Err(NetworkError::RpcTimeout);
            }
        }
        Err(NetworkError::RpcTimeout)
    }

    fn send_encrypted_message(
        &mut self,
        body: &[u8],
        content_related: bool,
    ) -> Result<i64, NetworkError> {
        let message_id = self.next_message_id()?;
        let mut padding = vec![0_u8; encrypted_padding_length(body.len())];
        secure_random(&mut padding)?;
        let message = PlainMessage {
            server_salt: self.server_salt,
            session_id: self.session_id,
            message_id,
            sequence_number: self.sequence.next(content_related)?,
            body: body.to_vec(),
            padding,
        };
        let envelope = encrypt_message(&self.auth_key, CryptoDirection::ClientToServer, &message)?;
        self.transport.send_payload(&envelope.to_bytes())?;
        Ok(message_id)
    }

    fn send_acknowledgements(&mut self, message_ids: &[i64]) -> Result<(), NetworkError> {
        let body = fabushi_telegram_protocol::build_msgs_ack(message_ids)
            .map_err(|_| NetworkError::InvalidServiceMessage)?;
        self.send_encrypted_message(&body, false)?;
        Ok(())
    }

    fn reconcile_detailed_answer(
        &mut self,
        answer_message_id: i64,
        acknowledgements: &mut Vec<i64>,
    ) -> Result<(), NetworkError> {
        if self
            .message_ids
            .recent_server_message_ids()
            .contains(&answer_message_id)
        {
            if !acknowledgements.contains(&answer_message_id) {
                acknowledgements.push(answer_message_id);
            }
        } else {
            let resend = fabushi_telegram_protocol::build_msg_resend_req(&[answer_message_id])
                .map_err(|_| NetworkError::InvalidServiceMessage)?;
            self.send_encrypted_message(&resend, false)?;
        }
        Ok(())
    }

    fn scan_rpc_body(
        &mut self,
        body: &[u8],
        request_message_id: i64,
        state_query_message_id: Option<i64>,
        response_message_id: i64,
        acknowledgements: &mut Vec<i64>,
    ) -> Result<RpcOutcome, NetworkError> {
        match read_u32_at(body, 0)? {
            RPC_RESULT_CONSTRUCTOR => {
                let result = fabushi_telegram_protocol::parse_rpc_result(body)
                    .map_err(|_| NetworkError::InvalidServiceMessage)?;
                if !rpc_result_matches_request(result.request_message_id, request_message_id) {
                    return Ok(RpcOutcome::Continue);
                }
                Ok(RpcOutcome::Result(RpcResult {
                    request_message_id,
                    response_message_id,
                    body: result.result_body,
                }))
            }
            FUTURE_SALTS_CONSTRUCTOR => {
                let request_id = read_i64_at(body, 4)?;
                if request_id != request_message_id {
                    return Ok(RpcOutcome::Continue);
                }
                Ok(RpcOutcome::Result(RpcResult {
                    request_message_id,
                    response_message_id,
                    body: body.to_vec(),
                }))
            }
            MSGS_ACK_CONSTRUCTOR => {
                let message_ids = fabushi_telegram_protocol::parse_msgs_ack(body)
                    .map_err(|_| NetworkError::InvalidServiceMessage)?;
                // Fixed-U requestsAcked(..., byResponse=false) keeps callback RPCs
                // outstanding after a transport ACK. The single-RPC owner can
                // recognize the current id exactly, but only rpc_result or an
                // existing bounded recovery path may settle this call.
                let _acknowledges_current =
                    acknowledgement_mentions_request(&message_ids, request_message_id);
                Ok(RpcOutcome::Continue)
            }
            NEW_SESSION_CREATED_CONSTRUCTOR => {
                let first_message_id = read_i64_at(body, 4)?;
                self.server_salt = read_i64_at(body, 20)?;
                if new_session_requires_replay(request_message_id, first_message_id) {
                    Ok(RpcOutcome::RetryNewSession { first_message_id })
                } else {
                    Ok(RpcOutcome::Continue)
                }
            }
            MSGS_STATE_INFO_CONSTRUCTOR => {
                let info = fabushi_telegram_protocol::parse_msgs_state_info(body)
                    .map_err(|_| NetworkError::InvalidServiceMessage)?;
                if state_query_message_id != Some(info.request_message_id) {
                    return Ok(RpcOutcome::Continue);
                }
                let [state] = info.states.as_slice() else {
                    return Err(NetworkError::InvalidServiceMessage);
                };
                if request_state_requires_resend(*state) {
                    Ok(RpcOutcome::RetryState)
                } else {
                    Ok(RpcOutcome::Continue)
                }
            }
            MSGS_ALL_INFO_CONSTRUCTOR => {
                let info = fabushi_telegram_protocol::parse_msgs_all_info(body)
                    .map_err(|_| NetworkError::InvalidServiceMessage)?;
                if state_reconciliation_requires_resend(
                    &info.message_ids,
                    &info.states,
                    request_message_id,
                ) == Some(true)
                {
                    Ok(RpcOutcome::RetryState)
                } else {
                    Ok(RpcOutcome::Continue)
                }
            }
            MSG_DETAILED_INFO_CONSTRUCTOR => {
                let sent_message_id = read_i64_at(body, 4)?;
                let answer_message_id = read_i64_at(body, 12)?;
                if detailed_info_matches_request(sent_message_id, request_message_id) {
                    self.reconcile_detailed_answer(answer_message_id, acknowledgements)?;
                }
                Ok(RpcOutcome::Continue)
            }
            MSG_NEW_DETAILED_INFO_CONSTRUCTOR => {
                let answer_message_id = read_i64_at(body, 4)?;
                self.reconcile_detailed_answer(answer_message_id, acknowledgements)?;
                Ok(RpcOutcome::Continue)
            }
            BAD_SERVER_SALT_CONSTRUCTOR => {
                let message_id = read_i64_at(body, 4)?;
                let error_code = read_i32_at(body, 16)?;
                if message_id != request_message_id {
                    return Err(NetworkError::BadServerMessage {
                        message_id,
                        code: error_code,
                    });
                }
                self.server_salt = read_i64_at(body, 20)?;
                Ok(RpcOutcome::NewSalt)
            }
            BAD_MSG_NOTIFICATION_CONSTRUCTOR => {
                let message_id = read_i64_at(body, 4)?;
                let code = read_i32_at(body, 16)?;
                if message_id != request_message_id {
                    return Err(NetworkError::BadServerMessage { message_id, code });
                }
                match bad_message_recovery_action(code) {
                    Some(BadMessageRecoveryAction::CorrectTimeAndRetry) => {
                        self.correct_time_from_server_message(response_message_id)?;
                        Ok(RpcOutcome::RetryBadMessage { message_id, code })
                    }
                    Some(BadMessageRecoveryAction::ResetSessionAndRetry) => {
                        self.reset_session_state()?;
                        Ok(RpcOutcome::RetryBadMessage { message_id, code })
                    }
                    None => Err(NetworkError::BadServerMessage { message_id, code }),
                }
            },
            fabushi_telegram_protocol::MESSAGE_CONTAINER_CONSTRUCTOR => {
                let container = fabushi_telegram_protocol::parse_message_container(body)
                    .map_err(|_| NetworkError::InvalidServiceMessage)?;
                let mut outcome = RpcOutcome::Continue;
                for entry in container.entries {
                    self.validate_incoming_message_id(entry.message_id, &entry.body)?;
                    if contained_message_requires_ack(entry.sequence_number) {
                        acknowledgements.push(entry.message_id);
                    }
                    let inner_outcome = self.scan_rpc_body(
                        &entry.body,
                        request_message_id,
                        state_query_message_id,
                        entry.message_id,
                        acknowledgements,
                    )?;
                    if !matches!(inner_outcome, RpcOutcome::Continue) {
                        outcome = inner_outcome;
                    }
                }
                Ok(outcome)
            }
            _ => Ok(RpcOutcome::Continue),
        }
    }

    fn next_message_id(&mut self) -> Result<i64, NetworkError> {
        let (seconds, nanos) = unix_time()?;
        let adjusted = seconds
            .checked_add(self.clock_offset_seconds)
            .ok_or(NetworkError::InvalidSystemClock)?;
        Ok(self
            .message_ids
            .generate_client_message_id(adjusted, nanos)?)
    }

    fn validate_incoming_message_id(
        &mut self,
        message_id: i64,
        body: &[u8],
    ) -> Result<(), NetworkError> {
        let (local_now, _) = unix_time()?;
        let adjusted_now = local_now
            .checked_add(self.clock_offset_seconds)
            .ok_or(NetworkError::InvalidSystemClock)?;
        match self
            .message_ids
            .validate_server_message_id(message_id, adjusted_now)
        {
            Ok(()) => Ok(()),
            Err(MessageIdError::OutsideTimeWindow)
                if direct_bad_message_time_recovery(body) =>
            {
                self.message_ids
                    .validate_server_message_id(message_id, message_id >> 32)
                    .map_err(NetworkError::from)
            }
            Err(error) => Err(NetworkError::from(error)),
        }
    }

    fn correct_time_from_server_message(
        &mut self,
        response_message_id: i64,
    ) -> Result<(), NetworkError> {
        let server_seconds = response_message_id >> 32;
        let (local_seconds, _) = unix_time()?;
        self.clock_offset_seconds = server_seconds
            .checked_sub(local_seconds)
            .ok_or(NetworkError::InvalidSystemClock)?;
        self.server_time =
            i32::try_from(server_seconds).map_err(|_| NetworkError::InvalidSystemClock)?;
        self.message_ids.reset_client_message_id_floor();
        Ok(())
    }

    fn reset_session_state(&mut self) -> Result<(), NetworkError> {
        self.session_id = random_i64()?;
        self.sequence = SessionSequence::new();
        Ok(())
    }

    pub fn ping(&mut self) -> Result<PingResult, NetworkError> {
        let mut ping_bytes = [0_u8; 8];
        secure_random(&mut ping_bytes)?;
        self.send_ping(i64::from_le_bytes(ping_bytes))
    }

    pub fn send_ping(&mut self, ping_id: i64) -> Result<PingResult, NetworkError> {
        for attempt in 0..2 {
            let request_message_id = self.next_message_id()?;
            let mut body = Vec::with_capacity(12);
            body.extend_from_slice(&PING_CONSTRUCTOR.to_le_bytes());
            body.extend_from_slice(&ping_id.to_le_bytes());
            let mut padding = vec![0_u8; encrypted_padding_length(body.len())];
            secure_random(&mut padding)?;
            let message = PlainMessage {
                server_salt: self.server_salt,
                session_id: self.session_id,
                message_id: request_message_id,
                sequence_number: self.sequence.next(false)?,
                body,
                padding,
            };
            let envelope =
                encrypt_message(&self.auth_key, CryptoDirection::ClientToServer, &message)?;
            self.transport.send_payload(&envelope.to_bytes())?;

            let mut should_retry_with_new_salt = false;
            let mut retry_bad_message: Option<(i64, i32)> = None;
            for _ in 0..DEFAULT_MAX_SERVICE_MESSAGES {
                let payload = self.transport.receive_payload()?;
                let envelope = EncryptedEnvelope::from_bytes(&payload)?;
                let response = decrypt_message(
                    &self.auth_key,
                    CryptoDirection::ServerToClient,
                    &envelope,
                    self.session_id,
                )?;
                self.validate_incoming_message_id(response.message_id, &response.body)?;
                match self.process_service_body(
                    &response.body,
                    ping_id,
                    request_message_id,
                    response.message_id,
                )? {
                    ServiceOutcome::Pong(response_message_id) => {
                        return Ok(PingResult {
                            ping_id,
                            response_message_id,
                            server_salt: self.server_salt,
                        });
                    }
                    ServiceOutcome::NewSalt => should_retry_with_new_salt = true,
                    ServiceOutcome::RetryBadMessage { message_id, code } => {
                        retry_bad_message = Some((message_id, code));
                        break;
                    }
                    ServiceOutcome::Continue => {}
                }
            }
            if let Some((message_id, code)) = retry_bad_message {
                if attempt == 0 {
                    continue;
                }
                return Err(NetworkError::BadServerMessage { message_id, code });
            }
            if !(should_retry_with_new_salt && attempt == 0) {
                return Err(NetworkError::PongTimeout);
            }
        }
        Err(NetworkError::PongTimeout)
    }

    fn process_service_body(
        &mut self,
        body: &[u8],
        ping_id: i64,
        request_message_id: i64,
        response_message_id: i64,
    ) -> Result<ServiceOutcome, NetworkError> {
        match read_u32_at(body, 0)? {
            PONG_CONSTRUCTOR => {
                let pong = fabushi_telegram_protocol::parse_pong(body)
                    .map_err(|_| NetworkError::InvalidServiceMessage)?;
                if !pong_matches_request(
                    pong.message_id,
                    pong.ping_id,
                    request_message_id,
                    ping_id,
                ) {
                    return Err(NetworkError::PingMismatch);
                }
                Ok(ServiceOutcome::Pong(pong.message_id))
            }
            NEW_SESSION_CREATED_CONSTRUCTOR => {
                self.server_salt = read_i64_at(body, 20)?;
                Ok(ServiceOutcome::Continue)
            }
            BAD_SERVER_SALT_CONSTRUCTOR => {
                let message_id = read_i64_at(body, 4)?;
                let error_code = read_i32_at(body, 16)?;
                if message_id != request_message_id {
                    return Err(NetworkError::BadServerMessage {
                        message_id,
                        code: error_code,
                    });
                }
                self.server_salt = read_i64_at(body, 20)?;
                Ok(ServiceOutcome::NewSalt)
            }
            BAD_MSG_NOTIFICATION_CONSTRUCTOR => {
                let message_id = read_i64_at(body, 4)?;
                let code = read_i32_at(body, 16)?;
                if message_id != request_message_id {
                    return Err(NetworkError::BadServerMessage { message_id, code });
                }
                match bad_message_recovery_action(code) {
                    Some(BadMessageRecoveryAction::CorrectTimeAndRetry) => {
                        self.correct_time_from_server_message(response_message_id)?;
                        Ok(ServiceOutcome::RetryBadMessage { message_id, code })
                    }
                    Some(BadMessageRecoveryAction::ResetSessionAndRetry) => {
                        self.reset_session_state()?;
                        Ok(ServiceOutcome::RetryBadMessage { message_id, code })
                    }
                    None => Err(NetworkError::BadServerMessage { message_id, code }),
                }
            }
            fabushi_telegram_protocol::MESSAGE_CONTAINER_CONSTRUCTOR => {
                let container = fabushi_telegram_protocol::parse_message_container(body)
                    .map_err(|_| NetworkError::InvalidServiceMessage)?;
                let mut outcome = ServiceOutcome::Continue;
                for entry in container.entries {
                    self.validate_incoming_message_id(entry.message_id, &entry.body)?;
                    let inner_outcome = self.process_service_body(
                        &entry.body,
                        ping_id,
                        request_message_id,
                        entry.message_id,
                    )?;
                    if !matches!(inner_outcome, ServiceOutcome::Continue) {
                        outcome = inner_outcome;
                    }
                }
                Ok(outcome)
            }
            MSGS_ACK_CONSTRUCTOR => {
                let message_ids = fabushi_telegram_protocol::parse_msgs_ack(body)
                    .map_err(|_| NetworkError::InvalidServiceMessage)?;
                let _acknowledges_current =
                    acknowledgement_mentions_request(&message_ids, request_message_id);
                Ok(ServiceOutcome::Continue)
            }
            _ => Ok(ServiceOutcome::Continue),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceOutcome {
    Pong(i64),
    NewSalt,
    RetryBadMessage { message_id: i64, code: i32 },
    Continue,
}

enum RpcOutcome {
    Result(RpcResult),
    NewSalt,
    RetryBadMessage { message_id: i64, code: i32 },
    RetryNewSession { first_message_id: i64 },
    RetryState,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BadMessageRecoveryAction {
    CorrectTimeAndRetry,
    ResetSessionAndRetry,
}

fn new_session_requires_replay(request_message_id: i64, first_message_id: i64) -> bool {
    request_message_id < first_message_id
}

fn detailed_info_matches_request(sent_message_id: i64, request_message_id: i64) -> bool {
    sent_message_id == request_message_id
}

fn contained_message_requires_ack(sequence_number: i32) -> bool {
    sequence_number.rem_euclid(2) == 1
}

fn acknowledgement_mentions_request(
    message_ids: &[i64],
    request_message_id: i64,
) -> bool {
    message_ids.contains(&request_message_id)
}

fn pong_matches_request(
    response_message_id: i64,
    response_ping_id: i64,
    request_message_id: i64,
    ping_id: i64,
) -> bool {
    response_message_id == request_message_id && response_ping_id == ping_id
}

fn rpc_result_matches_request(
    response_request_message_id: i64,
    request_message_id: i64,
) -> bool {
    response_request_message_id == request_message_id
}

fn request_state_requires_resend(state: u8) -> bool {
    (state & 0x07) != 0x04
}

fn state_reconciliation_requires_resend(
    message_ids: &[i64],
    states: &[u8],
    request_message_id: i64,
) -> Option<bool> {
    message_ids
        .iter()
        .position(|message_id| *message_id == request_message_id)
        .map(|index| request_state_requires_resend(states[index]))
}

fn bad_message_recovery_action(code: i32) -> Option<BadMessageRecoveryAction> {
    match code {
        16 | 17 | 64 => Some(BadMessageRecoveryAction::CorrectTimeAndRetry),
        32 | 33 => Some(BadMessageRecoveryAction::ResetSessionAndRetry),
        _ => None,
    }
}

fn direct_bad_message_time_recovery(body: &[u8]) -> bool {
    read_u32_at(body, 0).ok() == Some(BAD_MSG_NOTIFICATION_CONSTRUCTOR)
        && matches!(read_i32_at(body, 16).ok(), Some(16 | 17 | 64))
}

fn clock_offset_seconds(server_time: i32) -> Result<i64, NetworkError> {
    let (local_seconds, _) = unix_time()?;
    i64::from(server_time)
        .checked_sub(local_seconds)
        .ok_or(NetworkError::InvalidSystemClock)
}

pub fn connect_directory(
    directory: &DcDirectory,
    dc_id: i32,
    purpose: DcPurpose,
    config: &NetworkConfig,
) -> Result<(ConnectedEndpoint, TcpTransport), NetworkError> {
    let candidates = directory.candidates(dc_id, purpose, config.prefer_ipv6)?;
    let mut attempts = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let address = SocketAddr::new(candidate.ip_address, candidate.port);
        match TcpTransport::connect(address, config.transport_mode, config) {
            Ok(transport) => {
                return Ok((ConnectedEndpoint { dc_id, address }, transport));
            }
            Err(error) => attempts.push(format!("{address}: {error}")),
        }
    }
    Err(NetworkError::ConnectFailed { attempts })
}

pub fn resume_auth_key(
    directory: &DcDirectory,
    dc_id: i32,
    auth_key: AuthKey,
    server_salt: i64,
    server_time: i32,
    config: &NetworkConfig,
) -> Result<EstablishedSession, NetworkError> {
    let (endpoint, transport) = connect_directory(directory, dc_id, DcPurpose::Main, config)?;
    Ok(EstablishedSession {
        endpoint,
        transport,
        auth_key,
        server_salt,
        server_time,
        message_ids: MessageIdGuard::default(),
        sequence: SessionSequence::new(),
        session_id: random_i64()?,
        clock_offset_seconds: 0,
    })
}

pub fn establish_auth_key(
    directory: &DcDirectory,
    dc_id: i32,
    config: &NetworkConfig,
) -> Result<EstablishedSession, NetworkError> {
    let (endpoint, mut transport) = connect_directory(directory, dc_id, DcPurpose::Main, config)?;
    let mut nonce = Zeroizing::new([0_u8; 16]);
    getrandom::getrandom(&mut nonce[..]).map_err(|_| NetworkError::RandomnessUnavailable)?;
    let mut handshake = AuthKeyHandshake::new(*nonce);
    let mut message_ids = MessageIdGuard::default();

    let request = handshake.begin(next_message_id(&mut message_ids)?)?;
    let response = exchange_and_validate(&mut transport, &request, &mut message_ids)?;
    handshake.receive_res_pq(response)?;

    let request = handshake.prepare_req_dh_params(
        next_message_id(&mut message_ids)?,
        dc_id,
        directory.test_mode(),
    )?;
    let response = exchange_and_validate(&mut transport, &request, &mut message_ids)?;
    let request = handshake.receive_server_dh(response, next_message_id(&mut message_ids)?)?;

    let response = exchange_and_validate(&mut transport, &request, &mut message_ids)?;
    match handshake.receive_dh_gen(response, next_message_id(&mut message_ids)?)? {
        DhGenAction::Established(established) => Ok(EstablishedSession {
            endpoint,
            transport,
            auth_key: established.auth_key,
            server_salt: established.server_salt,
            server_time: established.server_time,
            message_ids,
            sequence: SessionSequence::new(),
            session_id: random_i64()?,
            clock_offset_seconds: clock_offset_seconds(established.server_time)?,
        }),
        DhGenAction::Retry(retry) => {
            continue_dh_retries(endpoint, transport, &mut handshake, &mut message_ids, retry)
        }
    }
}

fn continue_dh_retries(
    endpoint: ConnectedEndpoint,
    mut transport: TcpTransport,
    handshake: &mut AuthKeyHandshake,
    message_ids: &mut MessageIdGuard,
    mut request: PlaintextEnvelope,
) -> Result<EstablishedSession, NetworkError> {
    for _ in 0..DEFAULT_MAX_DH_RETRIES {
        let response = exchange_and_validate(&mut transport, &request, message_ids)?;
        match handshake.receive_dh_gen(response, next_message_id(message_ids)?)? {
            DhGenAction::Established(established) => {
                return Ok(EstablishedSession {
                    endpoint,
                    transport,
                    auth_key: established.auth_key,
                    server_salt: established.server_salt,
                    server_time: established.server_time,
                    message_ids: message_ids.clone(),
                    sequence: SessionSequence::new(),
                    session_id: random_i64()?,
                    clock_offset_seconds: clock_offset_seconds(established.server_time)?,
                });
            }
            DhGenAction::Retry(retry) => request = retry,
        }
    }
    Err(NetworkError::DhRetryLimit)
}

fn exchange_and_validate(
    transport: &mut TcpTransport,
    request: &PlaintextEnvelope,
    message_ids: &mut MessageIdGuard,
) -> Result<PlaintextEnvelope, NetworkError> {
    let response = transport.exchange_plaintext(request)?;
    let (seconds, _) = unix_time()?;
    message_ids.validate_server_message_id(response.message_id, seconds)?;
    Ok(response)
}

fn next_message_id(message_ids: &mut MessageIdGuard) -> Result<i64, NetworkError> {
    let (seconds, nanos) = unix_time()?;
    Ok(message_ids.generate_client_message_id(seconds, nanos)?)
}

fn unix_time() -> Result<(i64, u32), NetworkError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| NetworkError::InvalidSystemClock)?;
    let seconds =
        i64::try_from(duration.as_secs()).map_err(|_| NetworkError::InvalidSystemClock)?;
    Ok((seconds, duration.subsec_nanos()))
}

fn encrypted_padding_length(body_length: usize) -> usize {
    let unpadded = 32 + body_length;
    let minimum = unpadded + 12;
    minimum.next_multiple_of(16) - unpadded
}

fn random_i64() -> Result<i64, NetworkError> {
    let mut bytes = [0_u8; 8];
    secure_random(&mut bytes)?;
    Ok(i64::from_le_bytes(bytes))
}

fn secure_random(output: &mut [u8]) -> Result<(), NetworkError> {
    getrandom::getrandom(output).map_err(|_| NetworkError::RandomnessUnavailable)
}

fn read_u32_at(input: &[u8], offset: usize) -> Result<u32, NetworkError> {
    Ok(u32::from_le_bytes(
        input
            .get(offset..offset + 4)
            .ok_or(NetworkError::InvalidServiceMessage)?
            .try_into()
            .expect("four-byte slice"),
    ))
}

fn read_i32_at(input: &[u8], offset: usize) -> Result<i32, NetworkError> {
    Ok(i32::from_le_bytes(
        input
            .get(offset..offset + 4)
            .ok_or(NetworkError::InvalidServiceMessage)?
            .try_into()
            .expect("four-byte slice"),
    ))
}

fn read_i64_at(input: &[u8], offset: usize) -> Result<i64, NetworkError> {
    Ok(i64::from_le_bytes(
        input
            .get(offset..offset + 8)
            .ok_or(NetworkError::InvalidServiceMessage)?
            .try_into()
            .expect("eight-byte slice"),
    ))
}

#[cfg(test)]
mod recovery_contract_tests {
    use super::{
        acknowledgement_mentions_request, bad_message_recovery_action,
        contained_message_requires_ack,
        direct_bad_message_time_recovery, detailed_info_matches_request,
        new_session_requires_replay, pong_matches_request, request_state_requires_resend,
        rpc_result_matches_request, state_reconciliation_requires_resend, BadMessageRecoveryAction,
        BAD_MSG_NOTIFICATION_CONSTRUCTOR,
    };

    fn bad_message(code: i32) -> Vec<u8> {
        let mut body = Vec::with_capacity(20);
        body.extend_from_slice(&BAD_MSG_NOTIFICATION_CONSTRUCTOR.to_le_bytes());
        body.extend_from_slice(&123_i64.to_le_bytes());
        body.extend_from_slice(&1_i32.to_le_bytes());
        body.extend_from_slice(&code.to_le_bytes());
        body
    }

    #[test]
    fn fixed_u_recoverable_bad_message_codes_are_bounded_and_typed() {
        assert_eq!(
            bad_message_recovery_action(16),
            Some(BadMessageRecoveryAction::CorrectTimeAndRetry)
        );
        assert_eq!(
            bad_message_recovery_action(17),
            Some(BadMessageRecoveryAction::CorrectTimeAndRetry)
        );
        assert_eq!(
            bad_message_recovery_action(32),
            Some(BadMessageRecoveryAction::ResetSessionAndRetry)
        );
        assert_eq!(
            bad_message_recovery_action(33),
            Some(BadMessageRecoveryAction::ResetSessionAndRetry)
        );
        assert_eq!(
            bad_message_recovery_action(64),
            Some(BadMessageRecoveryAction::CorrectTimeAndRetry)
        );
        assert_eq!(bad_message_recovery_action(48), None);
        assert_eq!(bad_message_recovery_action(99), None);
    }

    #[test]
    fn new_session_replays_only_requests_older_than_first_message() {
        assert!(new_session_requires_replay(100, 104));
        assert!(!new_session_requires_replay(104, 104));
        assert!(!new_session_requires_replay(108, 104));
    }

    #[test]
    fn fixed_u_container_sequence_parity_drives_current_ack_accumulation() {
        assert!(!contained_message_requires_ack(0));
        assert!(contained_message_requires_ack(1));
        assert!(!contained_message_requires_ack(2));
        assert!(contained_message_requires_ack(3));
    }

    #[test]
    fn fixed_u_ack_recognizes_only_the_current_outstanding_rpc() {
        assert!(acknowledgement_mentions_request(&[100, 200, 300], 200));
        assert!(!acknowledgement_mentions_request(&[100, 200, 300], 400));
        assert!(!acknowledgement_mentions_request(&[], 200));
    }

    #[test]
    fn fixed_u_pong_matches_only_the_current_outstanding_ping() {
        assert!(pong_matches_request(100, 200, 100, 200));
        assert!(!pong_matches_request(101, 200, 100, 200));
        assert!(!pong_matches_request(100, 201, 100, 200));
    }

    #[test]
    fn fixed_u_rpc_result_settles_only_the_current_outstanding_rpc() {
        assert!(rpc_result_matches_request(123, 123));
        assert!(!rpc_result_matches_request(122, 123));
        assert!(!rpc_result_matches_request(124, 123));
    }

    #[test]
    fn detailed_info_requests_resend_only_for_the_current_outstanding_rpc() {
        assert!(detailed_info_matches_request(123, 123));
        assert!(!detailed_info_matches_request(122, 123));
        assert!(!detailed_info_matches_request(124, 123));
    }

    #[test]
    fn fixed_u_message_state_low_bits_drive_bounded_resend() {
        assert!(request_state_requires_resend(0x00));
        assert!(request_state_requires_resend(0x01));
        assert!(request_state_requires_resend(0x03));
        assert!(!request_state_requires_resend(0x04));
        assert!(!request_state_requires_resend(0x0c));
        assert!(request_state_requires_resend(0x05));
    }

    #[test]
    fn all_info_reconciles_only_the_current_outstanding_rpc() {
        let ids = [100_i64, 200_i64];
        let states = [0x04_u8, 0x01_u8];
        assert_eq!(
            state_reconciliation_requires_resend(&ids, &states, 100),
            Some(false)
        );
        assert_eq!(
            state_reconciliation_requires_resend(&ids, &states, 200),
            Some(true)
        );
        assert_eq!(
            state_reconciliation_requires_resend(&ids, &states, 300),
            None
        );
    }

    #[test]
    fn only_direct_time_skew_notifications_relax_the_clock_window() {
        assert!(direct_bad_message_time_recovery(&bad_message(16)));
        assert!(direct_bad_message_time_recovery(&bad_message(17)));
        assert!(!direct_bad_message_time_recovery(&bad_message(32)));
        assert!(direct_bad_message_time_recovery(&bad_message(64)));
        assert!(!direct_bad_message_time_recovery(&[0_u8; 20]));
        assert!(!direct_bad_message_time_recovery(&[0_u8; 8]));
    }
}
