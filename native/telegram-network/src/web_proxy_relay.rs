use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
    thread,
    time::{Duration, Instant},
};

use fabushi_telegram_protocol::{
    parse_web_proxy_secret_text, validate_canonical_web_proxy_host, web_proxy_bridge_url,
};
use serde_json::Value;
use sha1::{Digest, Sha1};

use crate::{NetworkAttemptControl, NetworkConfig, NetworkError, NetworkProxyEndpoint, NetworkProxyKind};

const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_LOCAL_WEBSOCKET_PAYLOAD: usize = 2 * 1024 * 1024;
const MAX_FRAME_PAYLOAD: usize = 1024 * 1024;
const MAX_FRAMES_PER_BATCH: usize = 4096;
const DATA_FRAME_BYTES: usize = 64 * 1024;
const INITIAL_STREAM_WINDOW: u32 = 4 * 1024 * 1024;
const WINDOW_FLUSH_BYTES: u32 = 256 * 1024;
const WINDOW_FLUSH_DELAY: Duration = Duration::from_millis(20);
const CAPABILITY_LIFETIME: Duration = Duration::from_secs(5 * 60);
const LOCAL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const WELCOME_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CLOSED_STREAM_IDS: usize = 4096;
const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebProxyBootstrap {
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameType {
    Open = 0x01,
    Data = 0x02,
    Close = 0x03,
    Window = 0x04,
    Ping = 0x05,
    Pong = 0x06,
    Hello = 0x10,
    Welcome = 0x11,
    AuthChallenge = 0x12,
    AuthResponse = 0x13,
    Bye = 0x1f,
}

impl FrameType {
    fn parse(value: u8) -> Option<Self> {
        Some(match value {
            0x01 => Self::Open,
            0x02 => Self::Data,
            0x03 => Self::Close,
            0x04 => Self::Window,
            0x05 => Self::Ping,
            0x06 => Self::Pong,
            0x10 => Self::Hello,
            0x11 => Self::Welcome,
            0x12 => Self::AuthChallenge,
            0x13 => Self::AuthResponse,
            0x1f => Self::Bye,
            _ => return None,
        })
    }
}

#[derive(Debug)]
struct RelayFrame {
    kind: FrameType,
    stream_id: u32,
    payload: Vec<u8>,
}

fn serialize_frame(kind: FrameType, stream_id: u32, payload: &[u8]) -> io::Result<Vec<u8>> {
    if stream_id > 0x00ff_ffff || payload.len() > MAX_FRAME_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid WebProxy relay frame"));
    }
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "WebProxy relay frame too large"))?;
    let mut out = Vec::with_capacity(8 + payload.len());
    out.push(kind as u8);
    out.push((stream_id >> 16) as u8);
    out.push((stream_id >> 8) as u8);
    out.push(stream_id as u8);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

fn parse_available_frames(input: &mut Vec<u8>) -> io::Result<Vec<RelayFrame>> {
    let mut result = Vec::new();
    let mut consumed = 0_usize;
    while input.len().saturating_sub(consumed) >= 8 {
        let header = &input[consumed..];
        let kind = FrameType::parse(header[0])
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unknown WebProxy relay frame"))?;
        let stream_id = (u32::from(header[1]) << 16)
            | (u32::from(header[2]) << 8)
            | u32::from(header[3]);
        let payload_len = u32::from_be_bytes(header[4..8].try_into().expect("relay header")) as usize;
        if payload_len > MAX_FRAME_PAYLOAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid WebProxy relay payload length"));
        }
        if header.len() < 8 + payload_len {
            break;
        }
        if result.len() >= MAX_FRAMES_PER_BATCH {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "too many WebProxy relay frames"));
        }
        result.push(RelayFrame {
            kind,
            stream_id,
            payload: header[8..8 + payload_len].to_vec(),
        });
        consumed += 8 + payload_len;
    }
    if consumed != 0 {
        input.drain(..consumed);
    }
    if input.len() > 8 + MAX_FRAME_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "WebProxy relay input buffer exceeded"));
    }
    Ok(result)
}

fn window_payload(amount: u32) -> [u8; 4] {
    amount.to_be_bytes()
}

fn read_window(payload: &[u8]) -> io::Result<u32> {
    if payload.len() != 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "malformed WebProxy window"));
    }
    let value = u32::from_be_bytes(payload.try_into().expect("window payload"));
    if value == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "zero WebProxy window"));
    }
    Ok(value)
}

#[derive(Default)]
struct CarrierState {
    writer: Option<TcpStream>,
    input: Vec<u8>,
    welcomed: bool,
    failed: bool,
    generation: u64,
}

struct StreamState {
    inner: Mutex<StreamInner>,
    wake: Condvar,
}

struct StreamInner {
    incoming: VecDeque<u8>,
    send_window: u64,
    receive_window: u32,
    pending_window: u32,
    window_flush_scheduled: bool,
    opened: bool,
    closed: bool,
    failed: bool,
}

impl Default for StreamInner {
    fn default() -> Self {
        Self {
            incoming: VecDeque::new(),
            send_window: u64::from(INITIAL_STREAM_WINDOW),
            receive_window: INITIAL_STREAM_WINDOW,
            pending_window: 0,
            window_flush_scheduled: false,
            opened: false,
            closed: false,
            failed: false,
        }
    }
}

struct RelayInner {
    carrier: CarrierState,
    streams: BTreeMap<u32, Arc<StreamState>>,
    closed: VecDeque<u32>,
    next_stream_id: u32,
    bootstrap_token: String,
    bootstrap_expires: Instant,
}

struct WebProxyRelay {
    remote_url: String,
    listener_address: SocketAddr,
    inner: Mutex<RelayInner>,
    carrier_wake: Condvar,
}

static RELAYS: OnceLock<Mutex<BTreeMap<String, Arc<WebProxyRelay>>>> = OnceLock::new();

fn relay_registry() -> &'static Mutex<BTreeMap<String, Arc<WebProxyRelay>>> {
    RELAYS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn endpoint_key(endpoint: &NetworkProxyEndpoint) -> Result<String, NetworkError> {
    if endpoint.kind != NetworkProxyKind::Web
        || endpoint.port != 443
        || endpoint.host.trim() != endpoint.host
        || endpoint.host.is_empty()
    {
        return Err(NetworkError::InvalidProxyEndpoint);
    }
    validate_canonical_web_proxy_host(&endpoint.host)
        .map_err(|_| NetworkError::InvalidProxyEndpoint)?;
    parse_web_proxy_secret_text(&endpoint.password)
        .map_err(|_| NetworkError::InvalidProxySecret)?;
    web_proxy_bridge_url(&endpoint.host, &endpoint.user, &endpoint.password)
        .map_err(|_| NetworkError::InvalidProxyEndpoint)
}

fn relay_for(endpoint: &NetworkProxyEndpoint) -> Result<Arc<WebProxyRelay>, NetworkError> {
    let remote_url = endpoint_key(endpoint)?;
    let mut registry = relay_registry()
        .lock()
        .map_err(|_| NetworkError::WebProxyRelayUnavailable)?;
    if let Some(existing) = registry.get(&remote_url) {
        return Ok(Arc::clone(existing));
    }
    let relay = WebProxyRelay::start(remote_url.clone())?;
    registry.insert(remote_url, Arc::clone(&relay));
    Ok(relay)
}

pub fn prepare_web_proxy_relay(
    endpoint: &NetworkProxyEndpoint,
) -> Result<WebProxyBootstrap, NetworkError> {
    let relay = relay_for(endpoint)?;
    relay.bootstrap()
}

pub fn retire_web_proxy_relay(endpoint: &NetworkProxyEndpoint) {
    let Ok(key) = endpoint_key(endpoint) else {
        return;
    };
    if let Ok(mut registry) = relay_registry().lock() {
        if let Some(relay) = registry.remove(&key) {
            relay.fail_all();
        }
    }
}

pub(crate) struct WebProxyRelayStream {
    relay: Arc<WebProxyRelay>,
    stream_id: u32,
    state: Arc<StreamState>,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

#[derive(Clone)]
pub(crate) struct WebProxyRelayCancel {
    relay: Weak<WebProxyRelay>,
    stream_id: u32,
}

impl WebProxyRelayCancel {
    pub(crate) fn cancel(&self) {
        if let Some(relay) = self.relay.upgrade() {
            relay.close_stream(self.stream_id, false);
        }
    }
}

impl WebProxyRelayStream {
    pub(crate) fn connect(
        endpoint: &NetworkProxyEndpoint,
        config: &NetworkConfig,
        control: Option<&NetworkAttemptControl>,
    ) -> Result<Self, NetworkError> {
        let relay = relay_for(endpoint)?;
        let wait = if let Some(control) = control {
            control.bounded_timeout(config.connect_timeout)?
        } else {
            config.connect_timeout
        };
        relay.wait_for_carrier(wait)?;
        let (stream_id, state) = relay.open_stream()?;
        let stream = Self {
            relay,
            stream_id,
            state,
            read_timeout: Some(config.read_timeout),
            write_timeout: Some(config.write_timeout),
        };
        if let Some(control) = control {
            control.register_web_stream(stream.cancel_handle())?;
        }
        Ok(stream)
    }

    pub(crate) fn cancel_handle(&self) -> WebProxyRelayCancel {
        WebProxyRelayCancel {
            relay: Arc::downgrade(&self.relay),
            stream_id: self.stream_id,
        }
    }

    pub(crate) fn set_read_timeout(&mut self, timeout: Option<Duration>) {
        self.read_timeout = timeout;
    }

    pub(crate) fn set_write_timeout(&mut self, timeout: Option<Duration>) {
        self.write_timeout = timeout;
    }

    pub(crate) fn shutdown(&self) {
        self.relay.close_stream(self.stream_id, false);
    }
}

impl Drop for WebProxyRelayStream {
    fn drop(&mut self) {
        self.relay.close_stream(self.stream_id, false);
    }
}

impl Read for WebProxyRelayStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let deadline = self.read_timeout.map(|timeout| Instant::now() + timeout);
        let mut guard = self.state.inner.lock()
            .map_err(|_| io::Error::other("WebProxy stream state poisoned"))?;
        loop {
            if !guard.incoming.is_empty() {
                let count = output.len().min(guard.incoming.len());
                for byte in output.iter_mut().take(count) {
                    *byte = guard.incoming.pop_front().expect("incoming byte");
                }
                if u32::try_from(count).ok().is_none() {
                    return Err(io::Error::other("WebProxy read size overflow"));
                }
                let grant = count as u32;
                if grant > INITIAL_STREAM_WINDOW - guard.receive_window {
                    drop(guard);
                    self.relay.protocol_error();
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "WebProxy receive window overgrant"));
                }
                guard.receive_window += grant;
                guard.pending_window = guard.pending_window.saturating_add(grant);
                let immediate = guard.pending_window >= WINDOW_FLUSH_BYTES;
                let pending = if immediate {
                    guard.window_flush_scheduled = false;
                    std::mem::take(&mut guard.pending_window)
                } else {
                    if !guard.window_flush_scheduled {
                        guard.window_flush_scheduled = true;
                        self.relay.schedule_window_flush(self.stream_id, Arc::clone(&self.state));
                    }
                    0
                };
                drop(guard);
                if pending != 0 {
                    self.relay.send_frame(FrameType::Window, self.stream_id, &window_payload(pending))?;
                }
                return Ok(count);
            }
            if guard.failed {
                return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "WebProxy relay failed"));
            }
            if guard.closed {
                return Ok(0);
            }
            guard = wait_with_deadline(&self.state.wake, guard, deadline)?;
        }
    }
}

impl Write for WebProxyRelayStream {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if input.is_empty() {
            return Ok(0);
        }
        let deadline = self.write_timeout.map(|timeout| Instant::now() + timeout);
        let mut guard = self.state.inner.lock()
            .map_err(|_| io::Error::other("WebProxy stream state poisoned"))?;
        loop {
            if guard.failed {
                return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "WebProxy relay failed"));
            }
            if guard.closed {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "WebProxy relay stream closed"));
            }
            if guard.opened && guard.send_window != 0 {
                let count = input.len()
                    .min(DATA_FRAME_BYTES)
                    .min(usize::try_from(guard.send_window).unwrap_or(usize::MAX));
                guard.send_window -= count as u64;
                drop(guard);
                self.relay.send_frame(FrameType::Data, self.stream_id, &input[..count])?;
                return Ok(count);
            }
            guard = wait_with_deadline(&self.state.wake, guard, deadline)?;
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.relay.flush_carrier()
    }
}

fn wait_with_deadline<'a>(
    wake: &Condvar,
    guard: std::sync::MutexGuard<'a, StreamInner>,
    deadline: Option<Instant>,
) -> io::Result<std::sync::MutexGuard<'a, StreamInner>> {
    if let Some(deadline) = deadline {
        let remaining = deadline.checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "WebProxy relay timed out"))?;
        let (guard, timeout) = wake.wait_timeout(guard, remaining)
            .map_err(|_| io::Error::other("WebProxy stream wait poisoned"))?;
        if timeout.timed_out() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "WebProxy relay timed out"));
        }
        Ok(guard)
    } else {
        wake.wait(guard).map_err(|_| io::Error::other("WebProxy stream wait poisoned"))
    }
}

impl WebProxyRelay {
    fn start(remote_url: String) -> Result<Arc<Self>, NetworkError> {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .map_err(NetworkError::Io)?;
        let listener_address = listener.local_addr().map_err(NetworkError::Io)?;
        listener.set_nonblocking(true).map_err(NetworkError::Io)?;
        let token = random_url_token(32).map_err(NetworkError::Io)?;
        let relay = Arc::new(Self {
            remote_url,
            listener_address,
            inner: Mutex::new(RelayInner {
                carrier: CarrierState::default(),
                streams: BTreeMap::new(),
                closed: VecDeque::new(),
                next_stream_id: 1,
                bootstrap_token: token,
                bootstrap_expires: Instant::now() + CAPABILITY_LIFETIME,
            }),
            carrier_wake: Condvar::new(),
        });
        let weak = Arc::downgrade(&relay);
        thread::Builder::new()
            .name("fabushi-web-proxy-relay".into())
            .spawn(move || {
                while let Some(relay) = weak.upgrade() {
                    match listener.accept() {
                        Ok((socket, peer)) => {
                            if !peer.ip().is_loopback() {
                                let _ = socket.shutdown(Shutdown::Both);
                                continue;
                            }
                            let clone = Arc::clone(&relay);
                            let _ = thread::Builder::new()
                                .name("fabushi-web-proxy-client".into())
                                .spawn(move || clone.serve_local_client(socket));
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            drop(relay);
                            thread::sleep(Duration::from_millis(25));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|_| NetworkError::WebProxyRelayUnavailable)?;
        Ok(relay)
    }

    fn bootstrap(&self) -> Result<WebProxyBootstrap, NetworkError> {
        let token = random_url_token(32).map_err(NetworkError::Io)?;
        let mut guard = self.inner.lock().map_err(|_| NetworkError::WebProxyRelayUnavailable)?;
        guard.bootstrap_token = token;
        guard.bootstrap_expires = Instant::now() + CAPABILITY_LIFETIME;
        Ok(WebProxyBootstrap {
            url: format!("http://127.0.0.1:{}/#{}", self.listener_address.port(), guard.bootstrap_token),
        })
    }

    fn wait_for_carrier(&self, timeout: Duration) -> Result<(), NetworkError> {
        let deadline = Instant::now() + timeout.min(WELCOME_TIMEOUT);
        let mut guard = self.inner.lock().map_err(|_| NetworkError::WebProxyRelayUnavailable)?;
        loop {
            if guard.carrier.failed {
                return Err(NetworkError::WebProxyRelayUnavailable);
            }
            if guard.carrier.welcomed && guard.carrier.writer.is_some() {
                return Ok(());
            }
            let remaining = deadline.checked_duration_since(Instant::now())
                .ok_or(NetworkError::WebProxyWelcomeTimeout)?;
            let (next, timed) = self.carrier_wake.wait_timeout(guard, remaining)
                .map_err(|_| NetworkError::WebProxyRelayUnavailable)?;
            guard = next;
            if timed.timed_out() {
                return Err(NetworkError::WebProxyWelcomeTimeout);
            }
        }
    }

    fn open_stream(self: &Arc<Self>) -> Result<(u32, Arc<StreamState>), NetworkError> {
        let state = Arc::new(StreamState {
            inner: Mutex::new(StreamInner::default()),
            wake: Condvar::new(),
        });
        let (id, opened) = {
            let mut guard = self.inner.lock().map_err(|_| NetworkError::WebProxyRelayUnavailable)?;
            let mut id = guard.next_stream_id;
            for _ in 0..0x00ff_ffff {
                if id == 0 || id > 0x00ff_ffff {
                    id = 1;
                }
                if !guard.streams.contains_key(&id) && !guard.closed.contains(&id) {
                    break;
                }
                id += 1;
            }
            if guard.streams.contains_key(&id) || guard.closed.contains(&id) {
                return Err(NetworkError::WebProxyRelayUnavailable);
            }
            guard.next_stream_id = if id == 0x00ff_ffff { 1 } else { id + 1 };
            let opened = guard.carrier.welcomed;
            if let Ok(mut stream) = state.inner.lock() {
                stream.opened = opened;
            }
            guard.streams.insert(id, Arc::clone(&state));
            (id, opened)
        };
        if opened {
            self.send_frame(FrameType::Open, id, &[])?;
        }
        Ok((id, state))
    }

    fn close_stream(&self, stream_id: u32, failed: bool) {
        let state = {
            let mut guard = match self.inner.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            let state = guard.streams.remove(&stream_id);
            if state.is_some() {
                remember_closed(&mut guard.closed, stream_id);
            }
            state
        };
        if let Some(state) = state {
            let opened = state.inner.lock().ok().map(|stream| stream.opened).unwrap_or(false);
            if opened {
                let _ = self.send_frame(FrameType::Close, stream_id, &[]);
            }
            if let Ok(mut stream) = state.inner.lock() {
                stream.failed = failed;
                stream.closed = true;
            }
            state.wake.notify_all();
        }
    }

    fn schedule_window_flush(self: &Arc<Self>, stream_id: u32, state: Arc<StreamState>) {
        let relay = Arc::downgrade(self);
        let _ = thread::Builder::new()
            .name("fabushi-web-proxy-window".into())
            .spawn(move || {
                thread::sleep(WINDOW_FLUSH_DELAY);
                let Some(relay) = relay.upgrade() else { return; };
                let pending = {
                    let Ok(mut stream) = state.inner.lock() else { return; };
                    stream.window_flush_scheduled = false;
                    if !stream.opened || stream.closed || stream.failed {
                        stream.pending_window = 0;
                        return;
                    }
                    std::mem::take(&mut stream.pending_window)
                };
                if pending != 0 {
                    let _ = relay.send_frame(FrameType::Window, stream_id, &window_payload(pending));
                }
            });
    }

    fn send_frame(&self, kind: FrameType, stream_id: u32, payload: &[u8]) -> io::Result<()> {
        let frame = serialize_frame(kind, stream_id, payload)?;
        let mut guard = self.inner.lock()
            .map_err(|_| io::Error::other("WebProxy relay state poisoned"))?;
        let writer = guard.carrier.writer.as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "WebProxy browser carrier unavailable"))?;
        write_websocket_frame(writer, 0x02, &frame)
    }

    fn flush_carrier(&self) -> io::Result<()> {
        let mut guard = self.inner.lock()
            .map_err(|_| io::Error::other("WebProxy relay state poisoned"))?;
        let writer = guard.carrier.writer.as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "WebProxy browser carrier unavailable"))?;
        writer.flush()
    }

    fn protocol_error(&self) {
        self.fail_all();
    }

    fn fail_all(&self) {
        let streams = {
            let mut guard = match self.inner.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            if let Some(writer) = guard.carrier.writer.take() {
                let _ = writer.shutdown(Shutdown::Both);
            }
            guard.carrier.input.clear();
            guard.carrier.welcomed = false;
            guard.carrier.failed = true;
            guard.carrier.generation = guard.carrier.generation.wrapping_add(1);
            guard.closed.clear();
            std::mem::take(&mut guard.streams)
        };
        for (_, state) in streams {
            if let Ok(mut stream) = state.inner.lock() {
                stream.failed = true;
                stream.closed = true;
                stream.incoming.clear();
            }
            state.wake.notify_all();
        }
        self.carrier_wake.notify_all();
    }

    fn serve_local_client(self: Arc<Self>, mut socket: TcpStream) {
        let _ = socket.set_read_timeout(Some(LOCAL_HANDSHAKE_TIMEOUT));
        let _ = socket.set_write_timeout(Some(LOCAL_HANDSHAKE_TIMEOUT));
        let request = match read_http_request(&mut socket) {
            Ok(value) => value,
            Err(_) => return,
        };
        let expected_host = format!("127.0.0.1:{}", self.listener_address.port());
        if request.host != expected_host {
            let _ = write_http(&mut socket, "403 Forbidden", "text/plain", b"", "");
            return;
        }
        if request.path == "/" {
            let page = self.browser_page();
            let _ = write_http(
                &mut socket,
                "200 OK",
                "text/html; charset=utf-8",
                page.as_bytes(),
                "Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n",
            );
            return;
        }
        if request.path != "/transport"
            || !request.upgrade_websocket
            || request.websocket_key.as_deref().is_none()
        {
            let _ = write_http(&mut socket, "404 Not Found", "text/plain", b"", "");
            return;
        }
        let origin = format!("http://{expected_host}");
        if request.origin.as_deref() != Some(origin.as_str()) {
            let _ = write_http(&mut socket, "403 Forbidden", "text/plain", b"", "");
            return;
        }
        let key = request.websocket_key.expect("checked websocket key");
        let accept = websocket_accept(&key);
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        );
        if socket.write_all(response.as_bytes()).is_err() || socket.flush().is_err() {
            return;
        }

        let auth = match read_websocket_message(&mut socket) {
            Ok(Some(message)) if message.opcode == 0x01 => message.payload,
            _ => return,
        };
        let auth: Value = match serde_json::from_slice(&auth) {
            Ok(value) => value,
            Err(_) => return,
        };
        let token = auth.get("token").and_then(Value::as_str).unwrap_or("");
        let valid_type = auth.get("t").and_then(Value::as_str) == Some("auth");
        let valid = {
            let mut guard = match self.inner.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            if !valid_type
                || Instant::now() >= guard.bootstrap_expires
                || token.is_empty()
                || token != guard.bootstrap_token
            {
                false
            } else {
                guard.bootstrap_token.clear();
                true
            }
        };
        if !valid {
            let _ = write_websocket_frame(&mut socket, 0x08, &[0x03, 0xf0]);
            return;
        }

        let replace_active = self.inner.lock().ok()
            .map(|guard| guard.carrier.writer.is_some() || guard.carrier.welcomed)
            .unwrap_or(true);
        if replace_active {
            self.fail_all();
        }
        let writer = match socket.try_clone() {
            Ok(value) => value,
            Err(_) => return,
        };
        let generation = {
            let mut guard = match self.inner.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            guard.carrier.writer = Some(writer);
            guard.carrier.input.clear();
            guard.carrier.failed = false;
            guard.carrier.welcomed = false;
            guard.carrier.generation = guard.carrier.generation.wrapping_add(1);
            guard.carrier.generation
        };

        let control = serde_json::json!({"t":"bridge","url":self.remote_url}).to_string();
        if write_websocket_frame(&mut socket, 0x01, control.as_bytes()).is_err()
            || write_websocket_frame(
                &mut socket,
                0x02,
                &serialize_frame(FrameType::Hello, 0, &[1]).unwrap_or_default(),
            ).is_err()
        {
            self.fail_generation(generation);
            return;
        }
        let _ = socket.set_read_timeout(None);
        while let Ok(Some(message)) = read_websocket_message(&mut socket) {
            match message.opcode {
                0x02 => {
                    let frames = {
                        let mut guard = match self.inner.lock() {
                            Ok(value) => value,
                            Err(_) => return,
                        };
                        if guard.carrier.generation != generation {
                            return;
                        }
                        guard.carrier.input.extend_from_slice(&message.payload);
                        match parse_available_frames(&mut guard.carrier.input) {
                            Ok(frames) => frames,
                            Err(_) => {
                                drop(guard);
                                self.fail_generation(generation);
                                return;
                            }
                        }
                    };
                    for frame in frames {
                        if self.process_frame(generation, frame).is_err() {
                            self.fail_generation(generation);
                            return;
                        }
                    }
                }
                0x08 => break,
                0x09 => {
                    if write_websocket_frame(&mut socket, 0x0a, &message.payload).is_err() {
                        break;
                    }
                }
                0x0a => {}
                _ => {
                    self.fail_generation(generation);
                    return;
                }
            }
        }
        self.fail_generation(generation);
    }

    fn fail_generation(&self, generation: u64) {
        let matches = self.inner.lock().ok()
            .map(|guard| guard.carrier.generation == generation)
            .unwrap_or(false);
        if matches {
            self.fail_all();
        }
    }

    fn process_frame(&self, generation: u64, frame: RelayFrame) -> io::Result<()> {
        {
            let guard = self.inner.lock().map_err(|_| io::Error::other("WebProxy relay state poisoned"))?;
            if guard.carrier.generation != generation {
                return Ok(());
            }
            if !guard.carrier.welcomed
                && !(frame.kind == FrameType::Welcome && frame.stream_id == 0)
            {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "relay frame before Welcome"));
            }
        }

        if frame.stream_id == 0 {
            match frame.kind {
                FrameType::Welcome if frame.payload.is_empty() => {
                    let streams = {
                        let mut guard = self.inner.lock().map_err(|_| io::Error::other("WebProxy relay state poisoned"))?;
                        if guard.carrier.welcomed {
                            return Err(io::Error::new(io::ErrorKind::InvalidData, "duplicate Welcome"));
                        }
                        guard.carrier.welcomed = true;
                        guard.streams.iter().map(|(id, state)| (*id, Arc::clone(state))).collect::<Vec<_>>()
                    };
                    for (id, state) in streams {
                        {
                            let mut stream = state.inner.lock().map_err(|_| io::Error::other("WebProxy stream state poisoned"))?;
                            stream.opened = true;
                        }
                        self.send_frame(FrameType::Open, id, &[])?;
                        state.wake.notify_all();
                    }
                    self.carrier_wake.notify_all();
                    Ok(())
                }
                FrameType::Ping if frame.payload.len() <= 64 => {
                    self.send_frame(FrameType::Pong, 0, &frame.payload)
                }
                FrameType::Bye => Err(io::Error::new(io::ErrorKind::ConnectionAborted, "WebProxy relay sent Bye")),
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid stream-zero relay frame")),
            }
        } else {
            let state = {
                let guard = self.inner.lock().map_err(|_| io::Error::other("WebProxy relay state poisoned"))?;
                if let Some(state) = guard.streams.get(&frame.stream_id) {
                    Some(Arc::clone(state))
                } else if guard.closed.contains(&frame.stream_id) {
                    return match frame.kind {
                        FrameType::Data if !frame.payload.is_empty() => Ok(()),
                        FrameType::Window if read_window(&frame.payload).is_ok() => Ok(()),
                        FrameType::Close if frame.payload.is_empty() => Ok(()),
                        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid late relay frame")),
                    };
                } else {
                    None
                }
            }
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unknown WebProxy relay stream"))?;
            let mut stream = state.inner.lock().map_err(|_| io::Error::other("WebProxy stream state poisoned"))?;
            if !stream.opened {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "relay stream not opened"));
            }
            match frame.kind {
                FrameType::Data => {
                    if frame.payload.is_empty()
                        || frame.payload.len() > stream.receive_window as usize
                    {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "WebProxy receive window exceeded"));
                    }
                    stream.receive_window -= frame.payload.len() as u32;
                    stream.incoming.extend(frame.payload);
                    drop(stream);
                    state.wake.notify_all();
                    Ok(())
                }
                FrameType::Window => {
                    let amount = read_window(&frame.payload)?;
                    stream.send_window = stream.send_window
                        .saturating_add(u64::from(amount))
                        .min(u64::from(u32::MAX));
                    drop(stream);
                    state.wake.notify_all();
                    Ok(())
                }
                FrameType::Close if frame.payload.is_empty() => {
                    stream.closed = true;
                    drop(stream);
                    let mut guard = self.inner.lock().map_err(|_| io::Error::other("WebProxy relay state poisoned"))?;
                    guard.streams.remove(&frame.stream_id);
                    remember_closed(&mut guard.closed, frame.stream_id);
                    drop(guard);
                    state.wake.notify_all();
                    Ok(())
                }
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid WebProxy stream frame")),
            }
        }
    }

    fn browser_page(&self) -> String {
        let remote = serde_json::to_string(&self.remote_url).unwrap_or_else(|_| "\"\"".into());
        format!(
            r#"<!doctype html><meta charset="utf-8"><meta name="referrer" content="no-referrer">
<title>Fabushi Web Proxy</title><p id="state">Connecting…</p><iframe hidden></iframe><script>
(()=>{{const remote={remote},token=location.hash.slice(1);history.replaceState(null,'',location.pathname);
const ws=new WebSocket('ws://127.0.0.1:{port}/transport');ws.binaryType='arraybuffer';
const channel=new MessageChannel(),port=channel.port1,pending=[];let initialized=false,frame=null;
port.start();ws.onopen=()=>ws.send(JSON.stringify({{t:'auth',token,browser:navigator.userAgent.slice(0,160)}}));
ws.onmessage=e=>{{if(e.data instanceof ArrayBuffer){{if(initialized)port.postMessage(e.data,[e.data]);else pending.push(e.data);return}}
let c;try{{c=JSON.parse(e.data)}}catch{{return}}if(!c||c.t!=='bridge'||c.url!==remote||frame)return;
frame=document.querySelector('iframe');frame.sandbox='allow-scripts allow-same-origin';frame.referrerPolicy='no-referrer';
frame.onload=()=>{{frame.contentWindow.postMessage({{t:'tproxy-init',v:1}},new URL(remote).origin,[channel.port2]);initialized=true;
while(pending.length){{const d=pending.shift();port.postMessage(d,[d])}}}};frame.src=remote;}};
port.onmessage=e=>{{if(e.data instanceof ArrayBuffer&&ws.readyState===WebSocket.OPEN)ws.send(e.data);
else if(e.data&&e.data.t==='close')ws.close();}};ws.onclose=()=>document.getElementById('state').textContent='Disconnected';}})();
</script>"#,
            port = self.listener_address.port()
        )
    }
}

fn remember_closed(closed: &mut VecDeque<u32>, stream_id: u32) {
    if closed.contains(&stream_id) {
        return;
    }
    closed.push_back(stream_id);
    while closed.len() > MAX_CLOSED_STREAM_IDS {
        closed.pop_front();
    }
}

struct HttpRequest {
    path: String,
    host: String,
    origin: Option<String>,
    websocket_key: Option<String>,
    upgrade_websocket: bool,
}

fn read_http_request(socket: &mut TcpStream) -> io::Result<HttpRequest> {
    let mut input = Vec::new();
    let mut chunk = [0_u8; 2048];
    loop {
        let count = socket.read(&mut chunk)?;
        if count == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "local browser closed"));
        }
        input.extend_from_slice(&chunk[..count]);
        if input.len() > MAX_HTTP_HEADER_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "local browser header too large"));
        }
        if input.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let text = std::str::from_utf8(&input)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "local browser header is not UTF-8"))?;
    let mut lines = text.split("\r\n");
    let request = lines.next().unwrap_or("");
    let mut parts = request.split_whitespace();
    if parts.next() != Some("GET") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "local browser request is not GET"));
    }
    let path = parts.next().unwrap_or("").to_owned();
    if parts.next() != Some("HTTP/1.1") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "local browser request is not HTTP/1.1"));
    }
    let mut headers = BTreeMap::<String, String>::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "malformed local browser header"));
        };
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() || headers.insert(name, value.trim().to_owned()).is_some() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "duplicate local browser header"));
        }
    }
    let connection = headers.get("connection").map(String::as_str).unwrap_or("").to_ascii_lowercase();
    let upgrade = headers.get("upgrade").map(String::as_str).unwrap_or("").to_ascii_lowercase();
    Ok(HttpRequest {
        path,
        host: headers.remove("host").unwrap_or_default(),
        origin: headers.remove("origin"),
        websocket_key: headers.remove("sec-websocket-key"),
        upgrade_websocket: upgrade == "websocket"
            && connection.split(',').any(|part| part.trim() == "upgrade")
            && headers.get("sec-websocket-version").map(String::as_str) == Some("13"),
    })
}

fn write_http(
    socket: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    extra: &str,
) -> io::Result<()> {
    write!(
        socket,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    )?;
    socket.write_all(body)?;
    socket.flush()
}

struct WebSocketMessage {
    opcode: u8,
    payload: Vec<u8>,
}

fn read_websocket_message(socket: &mut TcpStream) -> io::Result<Option<WebSocketMessage>> {
    let mut header = [0_u8; 2];
    match socket.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    if header[0] & 0x70 != 0 || header[0] & 0x80 == 0 || header[1] & 0x80 == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid local WebSocket frame"));
    }
    let opcode = header[0] & 0x0f;
    let mut size = u64::from(header[1] & 0x7f);
    if size == 126 {
        let mut bytes = [0_u8; 2];
        socket.read_exact(&mut bytes)?;
        size = u64::from(u16::from_be_bytes(bytes));
        if size < 126 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "noncanonical WebSocket length"));
        }
    } else if size == 127 {
        let mut bytes = [0_u8; 8];
        socket.read_exact(&mut bytes)?;
        size = u64::from_be_bytes(bytes);
        if size <= u64::from(u16::MAX) || size & (1_u64 << 63) != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "noncanonical WebSocket length"));
        }
    }
    if size > MAX_LOCAL_WEBSOCKET_PAYLOAD as u64 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "local WebSocket payload too large"));
    }
    let mut mask = [0_u8; 4];
    socket.read_exact(&mut mask)?;
    let mut payload = vec![0_u8; size as usize];
    socket.read_exact(&mut payload)?;
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    Ok(Some(WebSocketMessage { opcode, payload }))
}

fn write_websocket_frame(socket: &mut TcpStream, opcode: u8, payload: &[u8]) -> io::Result<()> {
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | opcode);
    if payload.len() < 126 {
        header.push(payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        header.push(126);
        header.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    socket.write_all(&header)?;
    socket.write_all(payload)?;
    socket.flush()
}

fn websocket_accept(key: &str) -> String {
    let mut sha = Sha1::new();
    sha.update(key.as_bytes());
    sha.update(WEBSOCKET_GUID.as_bytes());
    base64_standard(&sha.finalize())
}

fn random_url_token(bytes: usize) -> io::Result<String> {
    let mut value = vec![0_u8; bytes];
    getrandom::getrandom(&mut value)
        .map_err(|_| io::Error::other("secure random unavailable"))?;
    Ok(base64_url_no_pad(&value))
}

fn base64_url_no_pad(value: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    base64_with_table(value, TABLE, false)
}

fn base64_standard(value: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    base64_with_table(value, TABLE, true)
}

fn base64_with_table(value: &[u8], table: &[u8; 64], padding: bool) -> String {
    let mut out = String::new();
    for chunk in value.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        out.push(table[(a >> 2) as usize] as char);
        out.push(table[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(table[(((b & 15) << 2) | (c >> 6)) as usize] as char);
        } else if padding {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(table[(c & 63) as usize] as char);
        } else if padding {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn exact_relay_frame_vectors_are_stable() {
        assert_eq!(
            serialize_frame(FrameType::Open, 1, &[]).unwrap(),
            [0x01, 0, 0, 1, 0, 0, 0, 0]
        );
        assert_eq!(
            serialize_frame(FrameType::Data, 0x00ff01, &[0xaa, 0xbb]).unwrap(),
            [0x02, 0, 0xff, 1, 0, 0, 0, 2, 0xaa, 0xbb]
        );
        assert_eq!(
            serialize_frame(FrameType::Window, 0x123456, &0x01020304_u32.to_be_bytes()).unwrap(),
            [0x04, 0x12, 0x34, 0x56, 0, 0, 0, 4, 1, 2, 3, 4]
        );
    }

    #[test]
    fn malformed_window_and_unknown_frame_fail_closed() {
        assert!(read_window(&[0, 0, 0, 0]).is_err());
        assert!(read_window(&[1, 2, 3]).is_err());
        let mut unknown = vec![0x7f, 0, 0, 1, 0, 0, 0, 0];
        assert!(parse_available_frames(&mut unknown).is_err());
        let mut partial = vec![0x01, 0, 0, 1, 0, 0];
        assert!(parse_available_frames(&mut partial).unwrap().is_empty());
        assert_eq!(partial.len(), 6);
    }

    #[test]
    fn browser_page_uses_message_port_v1_and_never_embeds_proxy_secret() {
        let relay = WebProxyRelay {
            remote_url: "https://proxy.example/?bridge=capability".into(),
            listener_address: "127.0.0.1:41234".parse().unwrap(),
            inner: Mutex::new(RelayInner {
                carrier: CarrierState::default(),
                streams: BTreeMap::new(),
                closed: VecDeque::new(),
                next_stream_id: 1,
                bootstrap_token: "local-token".into(),
                bootstrap_expires: Instant::now() + CAPABILITY_LIFETIME,
            }),
            carrier_wake: Condvar::new(),
        };
        let page = relay.browser_page();
        assert!(page.contains("tproxy-init"));
        assert!(page.contains("v:1"));
        assert!(page.contains("MessageChannel"));
        assert!(!page.contains("00112233445566778899aabbccddeeff"));
    }
}
