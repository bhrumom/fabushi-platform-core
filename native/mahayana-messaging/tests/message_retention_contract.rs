use fabushi_messaging_core::*;

fn envelope(actor_id: &ActorId, request_id: &str, command: ClientCommand) -> ClientEnvelope {
    ClientEnvelope::new(
        RequestContext {
            request_id: request_id.into(),
            device_id: "device:test".into(),
            actor_id: actor_id.clone(),
            session_id: "session:test".into(),
            sent_at_ms: 0,
        },
        command,
    )
}

fn owner_participant(actor_id: &ActorId) -> Participant {
    Participant {
        actor_id: actor_id.clone(),
        role: ParticipantRole::Owner,
        joined_at_ms: 1,
        muted_until_ms: None,
    }
}

fn sync_messages(events: &[ServerEnvelope]) -> Vec<Message> {
    events
        .iter()
        .find_map(|envelope| match &envelope.event {
            ServerEvent::SyncBatch { messages, .. } => Some(messages.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
fn retention_policy_is_revision_fenced_and_expiry_survives_restart_without_resurrection() {
    let owner_id = ActorId::new("human:owner");
    let conversation_id = ConversationId::new("chat:retention");
    let mut service = MessagingService::load(MemoryStateStore::default()).expect("load service");

    service
        .handle(
            envelope(
                &owner_id,
                "profile",
                ClientCommand::UpsertProfile {
                    actor: Actor::human(owner_id.0.clone(), "Owner"),
                },
            ),
            1_000,
        )
        .expect("upsert owner");

    let mut conversation = Conversation::direct(
        conversation_id.0.clone(),
        "Retention",
        vec![owner_participant(&owner_id)],
        1_000,
    );
    conversation.owner_id = Some(owner_id.clone());
    service
        .handle(
            envelope(
                &owner_id,
                "conversation",
                ClientCommand::CreateConversation {
                    conversation: conversation.clone(),
                },
            ),
            1_000,
        )
        .expect("create conversation");

    let policy_events = service
        .handle(
            envelope(
                &owner_id,
                "retention-1",
                ClientCommand::SetConversationMessageRetention {
                    conversation_id: conversation_id.clone(),
                    retention_seconds: Some(1),
                    expected_revision: 0,
                },
            ),
            1_100,
        )
        .expect("set retention policy");
    assert!(policy_events.iter().any(|event| matches!(
        &event.event,
        ServerEvent::ConversationMessageRetentionChanged {
            conversation_id: changed_id,
            retention_seconds: Some(1),
            revision: 1,
        } if changed_id == &conversation_id
    )));
    let canonical = service
        .engine()
        .state()
        .conversations
        .get(&conversation_id)
        .expect("conversation state");
    assert_eq!(canonical.message_retention_seconds, Some(1));
    assert_eq!(canonical.retention_revision, 1);

    let stale = service
        .handle(
            envelope(
                &owner_id,
                "retention-stale",
                ClientCommand::SetConversationMessageRetention {
                    conversation_id: conversation_id.clone(),
                    retention_seconds: Some(5),
                    expected_revision: 0,
                },
            ),
            1_200,
        )
        .expect_err("stale policy must fail closed");
    assert!(matches!(
        stale,
        MessagingServiceError::Engine(EngineError::MessageRetention(
            MessageRetentionError::StaleRevision {
                expected: 0,
                actual: 1
            }
        ))
    ));

    let sent = service
        .handle(
            envelope(
                &owner_id,
                "send-1",
                ClientCommand::SendMessage {
                    conversation_id: conversation_id.clone(),
                    client_message_id: ClientMessageId("client:retention:1".into()),
                    content: MessageContent::Text {
                        text: FormattedText::plain("expires"),
                    },
                    reply_to_message_id: None,
                    thread_root_message_id: None,
                    scheduled_at_ms: None,
                    silent: false,
                    protected_content: false,
                },
            ),
            2_000,
        )
        .expect("send retained message");
    let message = sent
        .iter()
        .find_map(|event| match &event.event {
            ServerEvent::MessageAdded { message } => Some(message.clone()),
            _ => None,
        })
        .expect("message added");
    assert_eq!(message.expires_at_ms, Some(3_000));
    assert!(!message.deleted);

    let before_expiry = service
        .handle(
            envelope(
                &owner_id,
                "sync-before",
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            2_999,
        )
        .expect("sync before expiry");
    assert!(sync_messages(&before_expiry)
        .iter()
        .any(|candidate| candidate.id == message.id && !candidate.deleted));

    let at_expiry = service
        .handle(
            envelope(
                &owner_id,
                "sync-expired",
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            3_000,
        )
        .expect("sync at expiry");
    assert!(sync_messages(&at_expiry)
        .iter()
        .all(|candidate| candidate.id != message.id));
    assert!(service
        .engine()
        .state()
        .messages
        .get(&conversation_id)
        .and_then(|messages| messages.get(&message.id))
        .is_some_and(|candidate| candidate.deleted));

    let store = service.into_store();
    let mut restarted = MessagingService::load(store).expect("restart from durable snapshot");
    let restarted_conversation = restarted
        .engine()
        .state()
        .conversations
        .get(&conversation_id)
        .expect("conversation after restart");
    assert_eq!(restarted_conversation.message_retention_seconds, Some(1));
    assert_eq!(restarted_conversation.retention_revision, 1);
    assert!(restarted
        .engine()
        .state()
        .messages
        .get(&conversation_id)
        .and_then(|messages| messages.get(&message.id))
        .is_some_and(|candidate| candidate.deleted));

    let after_restart = restarted
        .handle(
            envelope(
                &owner_id,
                "sync-restart",
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            4_000,
        )
        .expect("sync after restart");
    assert!(sync_messages(&after_restart)
        .iter()
        .all(|candidate| candidate.id != message.id));
}

#[test]
fn scheduled_message_expiry_starts_from_scheduled_send_time() {
    let owner_id = ActorId::new("human:scheduled-owner");
    let conversation_id = ConversationId::new("chat:scheduled-retention");
    let mut service = MessagingService::load(MemoryStateStore::default()).expect("load service");
    service
        .handle(
            envelope(
                &owner_id,
                "profile",
                ClientCommand::UpsertProfile {
                    actor: Actor::human(owner_id.0.clone(), "Owner"),
                },
            ),
            1_000,
        )
        .expect("upsert owner");
    let mut conversation = Conversation::direct(
        conversation_id.0.clone(),
        "Scheduled retention",
        vec![owner_participant(&owner_id)],
        1_000,
    );
    conversation.owner_id = Some(owner_id.clone());
    service
        .handle(
            envelope(
                &owner_id,
                "conversation",
                ClientCommand::CreateConversation { conversation },
            ),
            1_000,
        )
        .expect("create conversation");
    service
        .handle(
            envelope(
                &owner_id,
                "retention",
                ClientCommand::SetConversationMessageRetention {
                    conversation_id: conversation_id.clone(),
                    retention_seconds: Some(60),
                    expected_revision: 0,
                },
            ),
            1_100,
        )
        .expect("set retention");

    let sent = service
        .handle(
            envelope(
                &owner_id,
                "scheduled-send",
                ClientCommand::SendMessage {
                    conversation_id,
                    client_message_id: ClientMessageId("client:scheduled:1".into()),
                    content: MessageContent::Text {
                        text: FormattedText::plain("later"),
                    },
                    reply_to_message_id: None,
                    thread_root_message_id: None,
                    scheduled_at_ms: Some(100_000),
                    silent: false,
                    protected_content: false,
                },
            ),
            2_000,
        )
        .expect("queue scheduled message");
    let message = sent
        .iter()
        .find_map(|event| match &event.event {
            ServerEvent::MessageAdded { message } => Some(message),
            _ => None,
        })
        .expect("message added");
    assert_eq!(message.expires_at_ms, Some(160_000));
}
