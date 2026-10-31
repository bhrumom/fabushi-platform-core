use fabushi_messaging_core::*;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn context(actor_id: &str, device_id: &str, session_id: &str, request_id: &str) -> RequestContext {
    RequestContext {
        request_id: request_id.into(),
        device_id: device_id.into(),
        actor_id: ActorId::new(actor_id),
        session_id: session_id.into(),
        sent_at_ms: 1,
    }
}

fn envelope(actor_id: &str, request_id: &str, command: ClientCommand) -> ClientEnvelope {
    ClientEnvelope::new(
        context(actor_id, "desktop:1", "session:1", request_id),
        command,
    )
}

fn second_device_envelope(
    actor_id: &str,
    request_id: &str,
    command: ClientCommand,
) -> ClientEnvelope {
    ClientEnvelope::new(
        context(actor_id, "mobile:2", "session:2", request_id),
        command,
    )
}

fn participant(actor_id: &str, role: ParticipantRole) -> Participant {
    Participant {
        actor_id: ActorId::new(actor_id),
        role,
        joined_at_ms: 1,
        muted_until_ms: None,
    }
}

fn send_text(client_message_id: &str, text: &str) -> ClientCommand {
    ClientCommand::SendMessage {
        conversation_id: ConversationId::new("chat:direct"),
        client_message_id: ClientMessageId(client_message_id.into()),
        content: MessageContent::Text {
            text: FormattedText::plain(text),
        },
        reply_to_message_id: None,
        thread_root_message_id: None,
        scheduled_at_ms: None,
        silent: false,
        protected_content: false,
    }
}

fn setup_direct_conversation<S: MessagingStateStore>(service: &mut MessagingService<S>) {
    service
        .handle(
            envelope(
                "human:alice",
                "actor:alice",
                ClientCommand::UpsertProfile {
                    actor: Actor::human("human:alice", "Alice"),
                },
            ),
            10,
        )
        .expect("upsert alice");
    service
        .handle(
            envelope(
                "human:bob",
                "actor:bob",
                ClientCommand::UpsertProfile {
                    actor: Actor::human("human:bob", "Bob"),
                },
            ),
            11,
        )
        .expect("upsert bob");
    service
        .handle(
            envelope(
                "human:outsider",
                "actor:outsider",
                ClientCommand::UpsertProfile {
                    actor: Actor::human("human:outsider", "Outsider"),
                },
            ),
            12,
        )
        .expect("upsert outsider");
    service
        .handle(
            envelope(
                "human:alice",
                "conversation:create",
                ClientCommand::CreateConversation {
                    conversation: Conversation::direct(
                        "chat:direct",
                        "Alice and Bob",
                        vec![
                            participant("human:alice", ParticipantRole::Owner),
                            participant("human:bob", ParticipantRole::Member),
                        ],
                        13,
                    ),
                },
            ),
            13,
        )
        .expect("create direct conversation");
}

fn sync_cursor(events: &[ServerEnvelope]) -> String {
    events
        .iter()
        .rev()
        .find_map(|event| match &event.event {
            ServerEvent::SyncBatch {
                next_cursor: Some(cursor),
                ..
            } => Some(cursor.clone()),
            _ => None,
        })
        .expect("sync checkpoint cursor")
}

fn message_from_events(events: &[ServerEnvelope]) -> Message {
    events
        .iter()
        .rev()
        .find_map(|event| match &event.event {
            ServerEvent::MessageChanged { message } | ServerEvent::MessageAdded { message } => {
                Some(message.clone())
            }
            _ => None,
        })
        .expect("message event")
}

fn temporary_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "fabushi-delta-sync-{name}-{}-{nonce}.sqlite3",
        std::process::id()
    ))
}

fn remove_db(path: &PathBuf) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn initial_snapshot_limit_keeps_complete_navigation_metadata() {
    let mut service = MessagingService::load(MemoryStateStore::default()).expect("load service");
    setup_direct_conversation(&mut service);

    service
        .handle(
            envelope(
                "human:carol",
                "actor:carol",
                ClientCommand::UpsertProfile {
                    actor: Actor::human("human:carol", "Carol"),
                },
            ),
            14,
        )
        .expect("upsert carol");
    service
        .handle(
            envelope(
                "human:alice",
                "conversation:second",
                ClientCommand::CreateConversation {
                    conversation: Conversation::direct(
                        "chat:second",
                        "Alice and Carol",
                        vec![
                            participant("human:alice", ParticipantRole::Owner),
                            participant("human:carol", ParticipantRole::Member),
                        ],
                        15,
                    ),
                },
            ),
            15,
        )
        .expect("create second conversation");

    for (request_id, client_id, text, now) in [
        ("send:snapshot:one", "client:snapshot:one", "one", 20),
        ("send:snapshot:two", "client:snapshot:two", "two", 21),
    ] {
        service
            .handle(
                envelope("human:alice", request_id, send_text(client_id, text)),
                now,
            )
            .expect("seed snapshot message");
    }

    let events = service
        .handle(
            envelope(
                "human:alice",
                "sync:small-snapshot",
                ClientCommand::Sync {
                    cursor: None,
                    limit: 1,
                },
            ),
            30,
        )
        .expect("initial snapshot");

    let (actors, conversations, messages) = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            ServerEvent::SyncBatch {
                actors,
                conversations,
                messages,
                ..
            } => Some((actors, conversations, messages)),
            _ => None,
        })
        .expect("sync batch");

    assert_eq!(
        conversations.len(),
        2,
        "all visible conversation summaries must be present"
    );
    assert!(conversations
        .iter()
        .any(|item| item.id == ConversationId::new("chat:direct")));
    assert!(conversations
        .iter()
        .any(|item| item.id == ConversationId::new("chat:second")));
    assert_eq!(
        actors.len(),
        3,
        "all actors visible through those conversations must be present"
    );
    assert!(actors
        .iter()
        .any(|item| item.id == ActorId::new("human:alice")));
    assert!(actors
        .iter()
        .any(|item| item.id == ActorId::new("human:bob")));
    assert!(actors
        .iter()
        .any(|item| item.id == ActorId::new("human:carol")));
    assert_eq!(
        messages.len(),
        1,
        "heavy message payload must still honor the requested limit"
    );
}

#[test]
fn send_message_is_idempotent_and_server_acknowledged() {
    let mut service = MessagingService::load(MemoryStateStore::default()).expect("load service");
    setup_direct_conversation(&mut service);

    let first = service
        .handle(
            envelope(
                "human:alice",
                "send:first",
                send_text("client:idem", "hello"),
            ),
            20,
        )
        .expect("send first message");
    let first_message = message_from_events(&first);
    assert!(first_message.id.0.starts_with("msg:"));
    assert_eq!(first_message.delivery_state, DeliveryState::Sent);

    let cursor_after_first = service.cursor();
    let retry = service
        .handle(
            envelope(
                "human:alice",
                "send:retry",
                send_text("client:idem", "hello"),
            ),
            21,
        )
        .expect("retry idempotent message");
    let retry_message = message_from_events(&retry);
    assert_eq!(retry_message.id, first_message.id);
    assert_eq!(retry_message.delivery_state, DeliveryState::Sent);
    assert_eq!(service.cursor(), cursor_after_first);
    assert_eq!(
        service.engine().state().messages[&ConversationId::new("chat:direct")].len(),
        1
    );
}

#[test]
fn reusing_client_message_id_with_different_payload_is_rejected() {
    let mut service = MessagingService::load(MemoryStateStore::default()).expect("load service");
    setup_direct_conversation(&mut service);
    service
        .handle(
            envelope(
                "human:alice",
                "send:first",
                send_text("client:conflict", "one"),
            ),
            20,
        )
        .expect("send first message");

    let error = service
        .handle(
            envelope(
                "human:alice",
                "send:conflict",
                send_text("client:conflict", "two"),
            ),
            21,
        )
        .expect_err("conflicting retry must fail");
    assert!(matches!(
        error,
        MessagingServiceError::IdempotencyConflict(_)
    ));
}

#[test]
fn recipient_sync_marks_direct_message_delivered_and_mark_read_moves_it_to_read() {
    let mut service = MessagingService::load(MemoryStateStore::default()).expect("load service");
    setup_direct_conversation(&mut service);
    let sent = service
        .handle(
            envelope(
                "human:alice",
                "send:delivery",
                send_text("client:delivery", "hello bob"),
            ),
            20,
        )
        .expect("send message");
    let message_id = message_from_events(&sent).id;

    service
        .handle(
            envelope(
                "human:bob",
                "sync:delivery",
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            21,
        )
        .expect("recipient sync");
    assert_eq!(
        service.engine().state().messages[&ConversationId::new("chat:direct")][&message_id]
            .delivery_state,
        DeliveryState::Delivered
    );

    let read_events = service
        .handle(
            envelope(
                "human:bob",
                "read:delivery",
                ClientCommand::MarkRead {
                    conversation_id: ConversationId::new("chat:direct"),
                    message_id: message_id.clone(),
                },
            ),
            22,
        )
        .expect("mark read");
    assert!(read_events
        .iter()
        .any(|event| matches!(&event.event, ServerEvent::ReadChanged { .. })));
    assert_eq!(
        service.engine().state().messages[&ConversationId::new("chat:direct")][&message_id]
            .delivery_state,
        DeliveryState::Read
    );
}

#[test]
fn durable_delta_sync_survives_restart_and_is_audience_scoped() {
    let path = temporary_db("restart");
    let mut service =
        MessagingService::load(SqliteStateStore::new(&path)).expect("load sqlite service");
    setup_direct_conversation(&mut service);

    let bob_baseline = service
        .handle(
            envelope(
                "human:bob",
                "sync:bob:baseline",
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            20,
        )
        .expect("bob baseline sync");
    let bob_cursor = sync_cursor(&bob_baseline);

    let outsider_baseline = service
        .handle(
            envelope(
                "human:outsider",
                "sync:outsider:baseline",
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            21,
        )
        .expect("outsider baseline sync");
    let outsider_cursor = sync_cursor(&outsider_baseline);

    let sent = service
        .handle(
            envelope(
                "human:alice",
                "send:delta",
                send_text("client:delta", "after baseline"),
            ),
            22,
        )
        .expect("send delta message");
    let message_id = message_from_events(&sent).id;
    let cursor_after_send = service.cursor();
    drop(service);

    let mut restarted =
        MessagingService::load(SqliteStateStore::new(&path)).expect("restart sqlite service");
    assert_eq!(restarted.cursor(), cursor_after_send);

    let bob_delta = restarted
        .handle(
            second_device_envelope(
                "human:bob",
                "sync:bob:device2",
                ClientCommand::Sync {
                    cursor: Some(bob_cursor),
                    limit: 100,
                },
            ),
            23,
        )
        .expect("bob device2 delta sync");
    assert!(bob_delta.iter().any(|event| matches!(
        &event.event,
        ServerEvent::MessageAdded { message } | ServerEvent::MessageChanged { message }
            if message.id == message_id
    )));
    assert!(bob_delta.iter().any(|event| matches!(
        &event.event,
        ServerEvent::SyncBatch {
            next_cursor: Some(_),
            ..
        }
    )));

    let outsider_delta = restarted
        .handle(
            second_device_envelope(
                "human:outsider",
                "sync:outsider:device2",
                ClientCommand::Sync {
                    cursor: Some(outsider_cursor),
                    limit: 100,
                },
            ),
            24,
        )
        .expect("outsider delta sync");
    assert!(!outsider_delta.iter().any(|event| matches!(
        &event.event,
        ServerEvent::MessageAdded { .. } | ServerEvent::MessageChanged { .. }
    )));

    remove_db(&path);
}

#[test]
fn cursor_older_than_migrated_journal_floor_falls_back_to_scoped_full_sync() {
    let path = temporary_db("floor-fallback");
    let mut legacy = SqliteStateStore::new(&path);
    let mut state = MessagingState::default();
    state.actors.insert(
        ActorId::new("human:alice"),
        Actor::human("human:alice", "Alice"),
    );
    legacy
        .save(&MessagingSnapshot::new(state, 41, 100))
        .expect("seed snapshot without journal");
    drop(legacy);

    let mut service =
        MessagingService::load(SqliteStateStore::new(&path)).expect("load migrated service");
    let events = service
        .handle(
            envelope(
                "human:alice",
                "sync:old-cursor",
                ClientCommand::Sync {
                    cursor: Some("40".into()),
                    limit: 100,
                },
            ),
            101,
        )
        .expect("fallback full sync");
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0].event, ServerEvent::SyncBatch { .. }));
    assert_eq!(events[0].cursor.as_deref(), Some("41"));
    remove_db(&path);
}
