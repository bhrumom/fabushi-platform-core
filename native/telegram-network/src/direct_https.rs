use crate::{NetworkAttemptControl, NetworkError, NormalizedServiceRequest};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use std::{
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread,
    time::Duration,
};
use thiserror::Error;

pub const SERVICE_REQUEST_STAGGER: Duration = Duration::from_millis(800);
const HTTPS_PORT: u16 = 443;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum DirectHttpsError {
    #[error("direct HTTPS request requires https scheme")]
    UnsupportedScheme,
    #[error("direct HTTPS request contains invalid header or target bytes")]
    InvalidRequest,
    #[error("direct HTTPS target did not resolve")]
    ResolutionFailed,
    #[error("direct HTTPS server name is invalid")]
    InvalidServerName,
    #[error("direct HTTPS response is malformed")]
    MalformedResponse,
    #[error("direct HTTPS response exceeded configured bound")]
    ResponseTooLarge,
    #[error("direct HTTPS I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("direct HTTPS TLS failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("direct HTTPS attempt was cancelled or exceeded its deadline: {0}")]
    Attempt(#[from] NetworkError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectHttpsResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl DirectHttpsResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaggerSettlementPolicy {
    DomainResolver,
    SpecialConfig,
}

#[derive(Debug)]
pub struct DirectHttpsReply {
    pub attempt_index: usize,
    pub result: Result<DirectHttpsResponse, DirectHttpsError>,
}

pub struct DirectHttpsRace {
    policy: StaggerSettlementPolicy,
    schedule_fenced: Arc<AtomicBool>,
    controls: Vec<Option<NetworkAttemptControl>>,
    replies: Receiver<DirectHttpsReply>,
}

impl DirectHttpsRace {
    pub fn start(
        policy: StaggerSettlementPolicy,
        requests: Vec<NormalizedServiceRequest>,
        total_attempt_timeout: Duration,
    ) -> Self {
        let schedule_fenced = Arc::new(AtomicBool::new(false));
        let (sender, replies) = mpsc::channel();
        let mut controls = Vec::with_capacity(requests.len());

        for (attempt_index, request) in requests.into_iter().enumerate() {
            let control = NetworkAttemptControl::new(total_attempt_timeout);
            controls.push(Some(control.clone()));
            let sender = sender.clone();
            let schedule_fenced = Arc::clone(&schedule_fenced);
            thread::spawn(move || {
                let stagger = SERVICE_REQUEST_STAGGER
                    .checked_mul(u32::try_from(attempt_index).unwrap_or(u32::MAX))
                    .unwrap_or(Duration::MAX);
                if !stagger.is_zero() {
                    thread::sleep(stagger);
                }
                if schedule_fenced.load(Ordering::Acquire) || control.is_cancelled() {
                    return;
                }
                let result = execute_direct_https_get(&request, &control);
                let _ = sender.send(DirectHttpsReply {
                    attempt_index,
                    result,
                });
            });
        }
        drop(sender);

        Self {
            policy,
            schedule_fenced,
            controls,
            replies,
        }
    }

    pub fn recv(&self) -> Result<DirectHttpsReply, mpsc::RecvError> {
        self.replies.recv()
    }

    pub fn try_recv(&self) -> Result<DirectHttpsReply, mpsc::TryRecvError> {
        self.replies.try_recv()
    }

    pub fn finalize_reply(&mut self, attempt_index: usize) {
        if let Some(slot) = self.controls.get_mut(attempt_index) {
            let _ = slot.take();
        }
    }

    pub fn cancel_reply(&mut self, attempt_index: usize) {
        if let Some(slot) = self.controls.get_mut(attempt_index) {
            if let Some(control) = slot.take() {
                control.cancel();
            }
        }
    }

    pub fn accept_reply(&mut self, attempt_index: usize) {
        self.finalize_reply(attempt_index);
        if self.policy == StaggerSettlementPolicy::DomainResolver {
            self.schedule_fenced.store(true, Ordering::Release);
            // fixed-U winner cleanup drops remaining reply wrappers without calling
            // ServiceWebRequest::destroy(); release ownership without explicit abort.
            for slot in &mut self.controls {
                let _ = slot.take();
            }
        }
    }

    pub fn schedule_is_fenced(&self) -> bool {
        self.schedule_fenced.load(Ordering::Acquire)
    }

    pub fn owned_request_count(&self) -> usize {
        self.controls.iter().filter(|slot| slot.is_some()).count()
    }
}

impl Drop for DirectHttpsRace {
    fn drop(&mut self) {
        self.schedule_fenced.store(true, Ordering::Release);
        for slot in &mut self.controls {
            if let Some(control) = slot.take() {
                control.cancel();
            }
        }
    }
}

pub fn execute_direct_https_get(
    request: &NormalizedServiceRequest,
    control: &NetworkAttemptControl,
) -> Result<DirectHttpsResponse, DirectHttpsError> {
    let result = execute_direct_https_get_inner(request, control);
    control.clear_stream();
    result
}

fn execute_direct_https_get_inner(
    request: &NormalizedServiceRequest,
    control: &NetworkAttemptControl,
) -> Result<DirectHttpsResponse, DirectHttpsError> {
    if request.scheme != "https" {
        return Err(DirectHttpsError::UnsupportedScheme);
    }
    validate_component(&request.url_host)?;
    validate_component(&request.path)?;
    if let Some(query) = request.query.as_deref() {
        validate_component(query)?;
    }
    if let Some(host) = request.host_override.as_deref() {
        validate_component(host)?;
    }
    validate_component(request.user_agent)?;
    if let Some(accept) = request.accept {
        validate_component(accept)?;
    }

    let mut last_error = None;
    let mut stream = None;
    for address in (request.url_host.as_str(), HTTPS_PORT).to_socket_addrs()? {
        let timeout = control.remaining()?;
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(candidate) => {
                stream = Some(candidate);
                break;
            }
            Err(error) => last_error = Some(error),
        }
    }
    let stream = match stream {
        Some(stream) => stream,
        None => {
            return Err(last_error
                .map(DirectHttpsError::Io)
                .unwrap_or(DirectHttpsError::ResolutionFailed))
        }
    };
    stream.set_nodelay(true)?;
    control.register_stream(&stream)?;
    stream.set_read_timeout(Some(control.remaining()?))?;
    stream.set_write_timeout(Some(control.remaining()?))?;

    let root_store = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.into(),
    };
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from(request.url_host.clone())
        .map_err(|_| DirectHttpsError::InvalidServerName)?;
    let connection = ClientConnection::new(Arc::new(config), server_name)?;
    let mut tls = StreamOwned::new(connection, stream);

    let request_bytes = serialize_get_request(request)?;
    tls.sock.set_write_timeout(Some(control.remaining()?))?;
    tls.write_all(&request_bytes)?;
    tls.sock.set_write_timeout(Some(control.remaining()?))?;
    tls.flush()?;

    let mut raw = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        tls.sock.set_read_timeout(Some(control.remaining()?))?;
        match tls.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                if raw.len().saturating_add(count) > MAX_RESPONSE_BYTES + MAX_HEADER_BYTES {
                    return Err(DirectHttpsError::ResponseTooLarge);
                }
                raw.extend_from_slice(&chunk[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(DirectHttpsError::Io(error)),
        }
    }
    parse_http_response(&raw)
}

fn serialize_get_request(
    request: &NormalizedServiceRequest,
) -> Result<Vec<u8>, DirectHttpsError> {
    let mut target = request.path.clone();
    if let Some(query) = request.query.as_deref() {
        target.push('?');
        target.push_str(query);
    }
    let host = request.host_override.as_deref().unwrap_or(&request.url_host);
    let mut serialized = format!(
        "GET {target} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {}\r\nConnection: close\r\nAccept-Encoding: identity\r\n",
        request.user_agent
    );
    if let Some(accept) = request.accept {
        serialized.push_str("Accept: ");
        serialized.push_str(accept);
        serialized.push_str("\r\n");
    }
    serialized.push_str("\r\n");
    Ok(serialized.into_bytes())
}

fn validate_component(value: &str) -> Result<(), DirectHttpsError> {
    if value.is_empty()
        || value
            .bytes()
            .any(|byte| byte == b'\r' || byte == b'\n' || byte == 0)
    {
        return Err(DirectHttpsError::InvalidRequest);
    }
    Ok(())
}

fn parse_http_response(raw: &[u8]) -> Result<DirectHttpsResponse, DirectHttpsError> {
    let header_end = find_bytes(raw, b"\r\n\r\n").ok_or(DirectHttpsError::MalformedResponse)?;
    if header_end > MAX_HEADER_BYTES {
        return Err(DirectHttpsError::ResponseTooLarge);
    }
    let header_text = std::str::from_utf8(&raw[..header_end])
        .map_err(|_| DirectHttpsError::MalformedResponse)?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().ok_or(DirectHttpsError::MalformedResponse)?;
    let mut status_parts = status_line.split_whitespace();
    let version = status_parts.next().ok_or(DirectHttpsError::MalformedResponse)?;
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(DirectHttpsError::MalformedResponse);
    }
    let status = status_parts
        .next()
        .ok_or(DirectHttpsError::MalformedResponse)?
        .parse::<u16>()
        .map_err(|_| DirectHttpsError::MalformedResponse)?;

    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or(DirectHttpsError::MalformedResponse)?;
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() {
            return Err(DirectHttpsError::MalformedResponse);
        }
        headers.push((name.to_owned(), value.to_owned()));
    }

    let body_bytes = &raw[header_end + 4..];
    let transfer_chunked = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("chunked"))
    });
    let body = if transfer_chunked {
        decode_chunked(body_bytes)?
    } else if let Some(length) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value)
    {
        let length = length
            .parse::<usize>()
            .map_err(|_| DirectHttpsError::MalformedResponse)?;
        if length > MAX_RESPONSE_BYTES || body_bytes.len() < length {
            return Err(if length > MAX_RESPONSE_BYTES {
                DirectHttpsError::ResponseTooLarge
            } else {
                DirectHttpsError::MalformedResponse
            });
        }
        body_bytes[..length].to_vec()
    } else {
        if body_bytes.len() > MAX_RESPONSE_BYTES {
            return Err(DirectHttpsError::ResponseTooLarge);
        }
        body_bytes.to_vec()
    };

    Ok(DirectHttpsResponse {
        status,
        headers,
        body,
    })
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>, DirectHttpsError> {
    let mut output = Vec::new();
    loop {
        let line_end = find_bytes(input, b"\r\n").ok_or(DirectHttpsError::MalformedResponse)?;
        let size_text = std::str::from_utf8(&input[..line_end])
            .map_err(|_| DirectHttpsError::MalformedResponse)?;
        let size_text = size_text.split(';').next().unwrap_or(size_text).trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| DirectHttpsError::MalformedResponse)?;
        input = &input[line_end + 2..];
        if size == 0 {
            return Ok(output);
        }
        if size > input.len() || input.len() < size + 2 || &input[size..size + 2] != b"\r\n" {
            return Err(DirectHttpsError::MalformedResponse);
        }
        if output.len().saturating_add(size) > MAX_RESPONSE_BYTES {
            return Err(DirectHttpsError::ResponseTooLarge);
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_normalization::{NormalizedServiceRequest, DNS_USER_AGENT};

    fn request() -> NormalizedServiceRequest {
        NormalizedServiceRequest {
            scheme: "https",
            url_host: "google.com".to_owned(),
            path: "/resolve".to_owned(),
            query: Some("name=example.com&type=1".to_owned()),
            host_override: Some("dns.google.com".to_owned()),
            accept: Some("application/dns-json"),
            user_agent: DNS_USER_AGENT,
        }
    }

    #[test]
    fn serialized_request_is_direct_origin_form_and_preserves_normalized_headers() {
        let bytes = serialize_get_request(&request()).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.starts_with("GET /resolve?name=example.com&type=1 HTTP/1.1\r\n"));
        assert!(text.contains("\r\nHost: dns.google.com\r\n"));
        assert!(text.contains("\r\nAccept: application/dns-json\r\n"));
        assert!(text.contains("\r\nConnection: close\r\n"));
        assert!(text.contains("\r\nAccept-Encoding: identity\r\n"));
        assert!(!text.contains("Proxy-"));
    }

    #[test]
    fn stagger_constant_is_exactly_eight_hundred_milliseconds() {
        assert_eq!(SERVICE_REQUEST_STAGGER, Duration::from_millis(800));
    }

    #[test]
    fn non_success_status_keeps_readable_body_as_response_metadata() {
        let response = parse_http_response(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\n\r\nbody",
        )
        .unwrap();
        assert_eq!(response.status, 503);
        assert_eq!(response.body, b"body");
    }

    #[test]
    fn chunked_body_is_bounded_and_decoded() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\ntest\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(response.body, b"test");
    }

    #[test]
    fn domain_acceptance_fences_schedule_without_explicit_abort() {
        let (_sender, replies) = mpsc::channel();
        let control = NetworkAttemptControl::new(Duration::from_secs(1));
        let mut race = DirectHttpsRace {
            policy: StaggerSettlementPolicy::DomainResolver,
            schedule_fenced: Arc::new(AtomicBool::new(false)),
            controls: vec![Some(control.clone()), Some(control.clone())],
            replies,
        };
        race.accept_reply(0);
        assert!(race.schedule_is_fenced());
        assert_eq!(race.owned_request_count(), 0);
        assert!(!control.is_cancelled());
    }

    #[test]
    fn special_config_acceptance_keeps_remaining_schedule_owned() {
        let (_sender, replies) = mpsc::channel();
        let first = NetworkAttemptControl::new(Duration::from_secs(1));
        let second = NetworkAttemptControl::new(Duration::from_secs(1));
        let mut race = DirectHttpsRace {
            policy: StaggerSettlementPolicy::SpecialConfig,
            schedule_fenced: Arc::new(AtomicBool::new(false)),
            controls: vec![Some(first), Some(second.clone())],
            replies,
        };
        race.accept_reply(0);
        assert!(!race.schedule_is_fenced());
        assert_eq!(race.owned_request_count(), 1);
        assert!(!second.is_cancelled());
    }

    #[test]
    fn explicit_cancel_is_distinct_from_ordinary_finalization() {
        let (_sender, replies) = mpsc::channel();
        let first = NetworkAttemptControl::new(Duration::from_secs(1));
        let second = NetworkAttemptControl::new(Duration::from_secs(1));
        let mut race = DirectHttpsRace {
            policy: StaggerSettlementPolicy::SpecialConfig,
            schedule_fenced: Arc::new(AtomicBool::new(false)),
            controls: vec![Some(first.clone()), Some(second.clone())],
            replies,
        };
        race.finalize_reply(0);
        race.cancel_reply(1);
        assert!(!first.is_cancelled());
        assert!(second.is_cancelled());
    }
}
