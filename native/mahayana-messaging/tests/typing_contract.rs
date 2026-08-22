use fabushi_messaging_core::*;

fn context(actor_id: &str, request: &str) -> RequestContext {
    RequestContext {
        request_id: request.into(),
        device_id: format!("device:{actor_id}"),
        actor_id: ActorId::new(actor_id),
        session_id: format!("session:{actor_id}"),
        sent_at_ms: 1,
    }
}

fn direct_conversation() -> Conversation {
    Conversation::direct(
        "chat:typing",
        "Typing",
        vec![
            Participant {
                actor_id: ActorId::new("human:a"),
                role: ParticipantRole::Owner,
                joined_at_ms: 1,
                muted_until_ms: None,
            },
            Participant {
                actor_id: ActorId::new("human:b"),
                role: ParticipantRole::Member,
                joined_at_ms: 1,
                muted_until_ms: None,
            },
        ],
        1,
    )
}

#[test]
fn typing_is_audience_scoped_and_expires_from_delta_replay() {
    let mut service = MessagingService::load(MemoryStateStore::default()).unwrap();
    for actor in ["human:a", "human:b", "human:outsider"] {
        service
            .handle(
                ClientEnvelope::new(
                    context(actor, "profile"),
                    ClientCommand::UpsertProfile {
                        actor: Actor::human(actor, actor),
                    },
                ),
                1,
            )
            .unwrap();
    }
    service
        .handle(
            ClientEnvelope::new(
                context("human:a", "conversation"),
                ClientCommand::CreateConversation {
                    conversation: direct_conversation(),
                },
            ),
            2,
        )
        .unwrap();
    let before = service.cursor();
    let start = service
        .handle(
            ClientEnvelope::new(
                context("human:a", "typing"),
                ClientCommand::StartTyping {
                    conversation_id: ConversationId::new("chat:typing"),
                    action: "typing".into(),
                },
            ),
            100,
        )
        .unwrap();
    assert!(
        matches!(&start[0].event, ServerEvent::TypingChanged { actor_id, action: Some(action), expires_at_ms: Some(5100), .. } if actor_id == &ActorId::new("human:a") && action == "typing")
    );

    let recipient = service
        .handle(
            ClientEnvelope::new(
                context("human:b", "sync-b"),
                ClientCommand::Sync {
                    cursor: Some(before.to_string()),
                    limit: 100,
                },
            ),
            200,
        )
        .unwrap();
    assert!(recipient.iter().any(|event| matches!(&event.event, ServerEvent::TypingChanged { actor_id, action: Some(_), .. } if actor_id == &ActorId::new("human:a"))));

    let outsider = service
        .handle(
            ClientEnvelope::new(
                context("human:outsider", "sync-o"),
                ClientCommand::Sync {
                    cursor: Some(before.to_string()),
                    limit: 100,
                },
            ),
            200,
        )
        .unwrap();
    assert!(!outsider
        .iter()
        .any(|event| matches!(event.event, ServerEvent::TypingChanged { .. })));

    let expired = service
        .handle(
            ClientEnvelope::new(
                context("human:b", "sync-expired"),
                ClientCommand::Sync {
                    cursor: Some(before.to_string()),
                    limit: 100,
                },
            ),
            6_000,
        )
        .unwrap();
    assert!(!expired.iter().any(|event| matches!(
        event.event,
        ServerEvent::TypingChanged {
            action: Some(_),
            ..
        }
    )));
}

#[test]
fn non_member_cannot_publish_typing_state() {
    let mut service = MessagingService::load(MemoryStateStore::default()).unwrap();
    for actor in ["human:a", "human:b", "human:outsider"] {
        service
            .handle(
                ClientEnvelope::new(
                    context(actor, "profile"),
                    ClientCommand::UpsertProfile {
                        actor: Actor::human(actor, actor),
                    },
                ),
                1,
            )
            .unwrap();
    }
    service
        .handle(
            ClientEnvelope::new(
                context("human:a", "conversation"),
                ClientCommand::CreateConversation {
                    conversation: direct_conversation(),
                },
            ),
            2,
        )
        .unwrap();
    let error = service
        .handle(
            ClientEnvelope::new(
                context("human:outsider", "typing"),
                ClientCommand::StartTyping {
                    conversation_id: ConversationId::new("chat:typing"),
                    action: "typing".into(),
                },
            ),
            100,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        MessagingServiceError::UnauthorizedCommand(_)
    ));
}
