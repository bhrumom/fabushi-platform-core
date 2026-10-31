use fabushi_messaging_core::{
    AccessGrant, AccessScope, Actor, ActorId, AuthenticatedClientFrame, ClientCommand,
    ClientEnvelope, ClientMessageId, Conversation, ConversationId, FileAccessTokenStore,
    FormattedText, MessageContent, MessagingServerConfig, MessagingWebSocketGateway,
    MessagingWebSocketGatewayConfig, Participant, ParticipantRole, RequestContext, ServerEvent,
    ServerFrame,
};
use std::collections::BTreeSet;
use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tungstenite::protocol::{Message, WebSocket};

struct Fixture {
    root: PathBuf,
    database: PathBuf,
    access_registry: PathBuf,
    actor_id: ActorId,
    device_id: String,
    session_id: String,
    access_token: String,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "fabushi-websocket-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        let database = root.join("messaging.sqlite3");
        let access_registry = root.join("access.json");
        let actor_id = ActorId::new(format!("human:{name}"));
        let device_id = format!("desktop:{name}");
        let session_id = format!("session:{name}");
        let store = FileAccessTokenStore::new(&access_registry);
        let issued = store
            .issue_random(AccessGrant {
                id: format!("grant:{name}"),
                actor_id: actor_id.clone(),
                device_id: device_id.clone(),
                session_id: session_id.clone(),
                scopes: BTreeSet::from([AccessScope::Messaging]),
                issued_at_ms: 1,
                expires_at_ms: None,
                revoked_at_ms: None,
            })
            .expect("issue access token");
        Self {
            root,
            database,
            access_registry,
            actor_id,
            device_id,
            session_id,
            access_token: issued.token,
        }
    }

    fn request(&self, session_id: &str) -> AuthenticatedClientFrame {
        AuthenticatedClientFrame {
            access_token: self.access_token.clone(),
            envelope: ClientEnvelope::new(
                RequestContext {
                    request_id: format!("request:{session_id}"),
                    device_id: self.device_id.clone(),
                    actor_id: self.actor_id.clone(),
                    session_id: session_id.to_string(),
                    sent_at_ms: 2,
                },
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn start_gateway(
    fixture: &Fixture,
    heartbeat_interval_ms: u64,
    heartbeat_timeout_ms: u64,
    max_frame_bytes: usize,
) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind gateway listener");
    let address = listener.local_addr().expect("gateway address");
    let mut server = MessagingServerConfig::new(
        address.to_string(),
        &fixture.database,
        &fixture.access_registry,
    );
    server.max_frame_bytes = max_frame_bytes;
    let gateway = MessagingWebSocketGateway::load(
        MessagingWebSocketGatewayConfig::new(server)
            .with_heartbeat(heartbeat_interval_ms, heartbeat_timeout_ms),
    )
    .expect("load gateway");
    let handle = thread::spawn(move || {
        gateway.serve_one(listener).expect("serve WebSocket client");
    });
    (address, handle)
}

fn connect(address: SocketAddr) -> WebSocket<TcpStream> {
    let stream = TcpStream::connect(address).expect("connect TCP");
    let (websocket, _) = tungstenite::client(format!("ws://{address}/messaging"), stream)
        .expect("WebSocket client handshake");
    websocket
}

fn authenticated_command(
    fixture: &Fixture,
    request_id: &str,
    command: ClientCommand,
) -> AuthenticatedClientFrame {
    AuthenticatedClientFrame {
        access_token: fixture.access_token.clone(),
        envelope: ClientEnvelope::new(
            RequestContext {
                request_id: request_id.into(),
                device_id: fixture.device_id.clone(),
                actor_id: fixture.actor_id.clone(),
                session_id: fixture.session_id.clone(),
                sent_at_ms: 0,
            },
            command,
        ),
    }
}

fn send_request(socket: &mut WebSocket<TcpStream>, request: &AuthenticatedClientFrame) {
    socket
        .send(Message::text(
            serde_json::to_string(request).expect("serialize request"),
        ))
        .expect("send request");
}

fn read_server_frame(socket: &mut WebSocket<TcpStream>) -> ServerFrame {
    loop {
        match socket.read().expect("read WebSocket frame") {
            Message::Text(text) => {
                return serde_json::from_str(text.as_str()).expect("parse server frame");
            }
            Message::Ping(_) => {
                socket.flush().expect("flush automatic pong");
            }
            Message::Pong(_) => {}
            other => panic!("unexpected WebSocket frame: {other:?}"),
        }
    }
}

fn close_and_join(mut socket: WebSocket<TcpStream>, handle: thread::JoinHandle<()>) {
    socket.close(None).expect("close WebSocket");
    handle.join().expect("join gateway");
}

#[test]
fn authorized_client_executes_protocol_v2_over_websocket() {
    let fixture = Fixture::new("authorized");
    let (address, handle) = start_gateway(&fixture, 500, 2_000, 8 * 1024 * 1024);
    let mut socket = connect(address);
    send_request(&mut socket, &fixture.request(&fixture.session_id));
    let frame = read_server_frame(&mut socket);
    assert!(matches!(frame, ServerFrame::Events { .. }));
    close_and_join(socket, handle);
}

#[test]
fn websocket_gateway_rejects_actor_device_session_mismatch() {
    let fixture = Fixture::new("unauthorized");
    let (address, handle) = start_gateway(&fixture, 500, 2_000, 8 * 1024 * 1024);
    let mut socket = connect(address);
    send_request(&mut socket, &fixture.request("wrong-session"));
    let frame = read_server_frame(&mut socket);
    assert!(matches!(
        frame,
        ServerFrame::Error { ref code, .. } if code == "unauthorized"
    ));
    close_and_join(socket, handle);
}

#[test]
fn websocket_gateway_keeps_connection_alive_with_ping_pong() {
    let fixture = Fixture::new("heartbeat");
    let (address, handle) = start_gateway(&fixture, 25, 500, 8 * 1024 * 1024);
    let mut socket = connect(address);

    let heartbeat = socket.read().expect("receive server heartbeat");
    assert!(matches!(heartbeat, Message::Ping(_)));
    socket.flush().expect("flush automatic heartbeat pong");

    send_request(&mut socket, &fixture.request(&fixture.session_id));
    let frame = read_server_frame(&mut socket);
    assert!(matches!(frame, ServerFrame::Events { .. }));
    close_and_join(socket, handle);
}

#[test]
fn websocket_gateway_rejects_oversized_application_frame() {
    let fixture = Fixture::new("frame-limit");
    let (address, handle) = start_gateway(&fixture, 500, 2_000, 1024);
    let mut socket = connect(address);
    socket
        .send(Message::text("x".repeat(1500)))
        .expect("send oversized application frame");
    let frame = read_server_frame(&mut socket);
    assert!(matches!(
        frame,
        ServerFrame::Error { ref code, .. } if code == "frame_too_large"
    ));
    close_and_join(socket, handle);
}

#[test]
fn websocket_gateway_rejects_binary_application_protocol_frames() {
    let fixture = Fixture::new("binary");
    let (address, handle) = start_gateway(&fixture, 500, 2_000, 8 * 1024 * 1024);
    let mut socket = connect(address);
    socket
        .send(Message::binary(b"not-json".to_vec()))
        .expect("send binary frame");
    let frame = read_server_frame(&mut socket);
    assert!(matches!(
        frame,
        ServerFrame::Error { ref code, .. } if code == "unsupported_binary"
    ));
    close_and_join(socket, handle);
}

#[allow(dead_code)]
fn assert_path_is_inside(root: &Path, child: &Path) {
    assert!(child.starts_with(root));
}


#[test]
fn websocket_retention_survives_sqlite_restart_and_does_not_resurrect_expired_message() {
    let fixture = Fixture::new("retention-restart");
    let conversation_id = ConversationId::new("chat:websocket-retention");

    let (address, first_handle) = start_gateway(&fixture, 500, 2_000, 8 * 1024 * 1024);
    let mut first = connect(address);

    send_request(
        &mut first,
        &authenticated_command(
            &fixture,
            "profile",
            ClientCommand::UpsertProfile {
                actor: Actor::human(fixture.actor_id.0.clone(), "Retention owner"),
            },
        ),
    );
    assert!(matches!(
        read_server_frame(&mut first),
        ServerFrame::Events { .. }
    ));

    let mut conversation = Conversation::direct(
        conversation_id.0.clone(),
        "WebSocket retention",
        vec![Participant {
            actor_id: fixture.actor_id.clone(),
            role: ParticipantRole::Owner,
            joined_at_ms: 1,
            muted_until_ms: None,
        }],
        1,
    );
    conversation.owner_id = Some(fixture.actor_id.clone());
    send_request(
        &mut first,
        &authenticated_command(
            &fixture,
            "conversation",
            ClientCommand::CreateConversation { conversation },
        ),
    );
    assert!(matches!(
        read_server_frame(&mut first),
        ServerFrame::Events { .. }
    ));

    send_request(
        &mut first,
        &authenticated_command(
            &fixture,
            "retention",
            ClientCommand::SetConversationMessageRetention {
                conversation_id: conversation_id.clone(),
                retention_seconds: Some(1),
                expected_revision: 0,
            },
        ),
    );
    let retention_frame = read_server_frame(&mut first);
    assert!(matches!(
        retention_frame,
        ServerFrame::Events { ref events }
            if events.iter().any(|event| matches!(
                &event.event,
                ServerEvent::ConversationMessageRetentionChanged {
                    conversation_id: changed_id,
                    retention_seconds: Some(1),
                    revision: 1,
                } if changed_id == &conversation_id
            ))
    ));

    // Use an authority-defined scheduled send time in the distant past. This avoids
    // wall-clock sleeps while still exercising the production expiry settlement path
    // on the next authenticated request.
    send_request(
        &mut first,
        &authenticated_command(
            &fixture,
            "send",
            ClientCommand::SendMessage {
                conversation_id: conversation_id.clone(),
                client_message_id: ClientMessageId("client:websocket-retention:1".into()),
                content: MessageContent::Text {
                    text: FormattedText::plain("expire through gateway"),
                },
                reply_to_message_id: None,
                thread_root_message_id: None,
                scheduled_at_ms: Some(1),
                silent: false,
                protected_content: false,
            },
        ),
    );
    let sent_frame = read_server_frame(&mut first);
    assert!(matches!(
        sent_frame,
        ServerFrame::Events { ref events }
            if events.iter().any(|event| matches!(
                &event.event,
                ServerEvent::MessageAdded { message }
                    if message.conversation_id == conversation_id
                        && message.expires_at_ms == Some(1_001)
                        && !message.deleted
            ))
    ));
    close_and_join(first, first_handle);

    // A new gateway instance loads the same SQLite authority and access registry.
    // Its first sync settles the overdue message through canonical tombstones before
    // projecting the snapshot, proving restart recovery without resurrection.
    let (address, restart_handle) = start_gateway(&fixture, 500, 2_000, 8 * 1024 * 1024);
    let mut restarted = connect(address);
    send_request(
        &mut restarted,
        &authenticated_command(
            &fixture,
            "sync-after-restart",
            ClientCommand::Sync {
                cursor: None,
                limit: 100,
            },
        ),
    );
    let frame = read_server_frame(&mut restarted);
    assert!(matches!(
        frame,
        ServerFrame::Events { ref events }
            if events.iter().any(|event| matches!(
                &event.event,
                ServerEvent::SyncBatch {
                    conversations,
                    messages,
                    ..
                } if conversations.iter().any(|conversation| {
                    conversation.id == conversation_id
                        && conversation.message_retention_seconds == Some(1)
                        && conversation.retention_revision == 1
                }) && messages.iter().all(|message| {
                    message.conversation_id != conversation_id || message.expires_at_ms != Some(1_001)
                })
            ))
    ));
    close_and_join(restarted, restart_handle);
}
