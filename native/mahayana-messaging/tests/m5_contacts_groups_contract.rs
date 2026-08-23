use fabushi_messaging_core::*;

fn context(actor_id: &str, request_id: &str) -> RequestContext {
    RequestContext {
        request_id: request_id.into(),
        device_id: format!("device:{actor_id}"),
        actor_id: ActorId::new(actor_id),
        session_id: format!("session:{actor_id}"),
        sent_at_ms: 1,
    }
}

fn participant(actor_id: &str, role: ParticipantRole) -> Participant {
    Participant {
        actor_id: ActorId::new(actor_id),
        role,
        joined_at_ms: 1,
        muted_until_ms: None,
    }
}

fn group_conversation(id: &str) -> Conversation {
    let mut conversation = Conversation::direct(
        id,
        "Dharma Study Group",
        vec![
            participant("human:owner", ParticipantRole::Owner),
            participant("human:admin", ParticipantRole::Admin),
            participant("human:member", ParticipantRole::Member),
            participant("human:restricted", ParticipantRole::Restricted),
        ],
        1,
    );
    conversation.kind = ConversationKind::Group;
    conversation.owner_id = Some(ActorId::new("human:owner"));
    conversation.permissions.can_send_messages = true;
    conversation.permissions.can_send_media = true;
    conversation.permissions.can_send_polls = true;
    conversation
}

fn community(id: &str) -> CommunityState {
    let mut community = CommunityState::new(ConversationId::new(id));
    community.upsert_member(CommunityMember {
        actor_id: ActorId::new("human:owner"),
        status: MemberStatus::Owner,
        admin_title: Some("Owner".into()),
        admin_rights: AdminRights::default(),
        restrictions: MemberRestrictions::default(),
        joined_at_ms: 1,
        invited_by: None,
    });
    community.upsert_member(CommunityMember {
        actor_id: ActorId::new("human:admin"),
        status: MemberStatus::Administrator,
        admin_title: Some("Invites".into()),
        admin_rights: AdminRights {
            invite_members: true,
            ..AdminRights::default()
        },
        restrictions: MemberRestrictions::default(),
        joined_at_ms: 1,
        invited_by: Some(ActorId::new("human:owner")),
    });
    community.upsert_member(CommunityMember {
        actor_id: ActorId::new("human:member"),
        status: MemberStatus::Member,
        admin_title: None,
        admin_rights: AdminRights::default(),
        restrictions: MemberRestrictions::default(),
        joined_at_ms: 1,
        invited_by: Some(ActorId::new("human:owner")),
    });
    community.upsert_member(CommunityMember {
        actor_id: ActorId::new("human:restricted"),
        status: MemberStatus::Restricted,
        admin_title: None,
        admin_rights: AdminRights::default(),
        restrictions: MemberRestrictions {
            send_messages: true,
            send_media: true,
            send_polls: true,
            ..MemberRestrictions::default()
        },
        joined_at_ms: 1,
        invited_by: Some(ActorId::new("human:owner")),
    });
    community
}

fn seed_engine(group_id: &str) -> MessagingEngine {
    let mut engine = MessagingEngine::new();
    for actor_id in [
        "human:owner",
        "human:admin",
        "human:member",
        "human:restricted",
    ] {
        engine
            .execute(Command::UpsertActor {
                actor: Actor::human(actor_id, actor_id),
            })
            .unwrap();
    }
    engine
        .execute(Command::UpsertConversation {
            conversation: group_conversation(group_id),
        })
        .unwrap();
    engine
        .execute(Command::UpdateCommunity {
            actor_id: ActorId::new("human:owner"),
            community: community(group_id),
        })
        .unwrap();
    engine
}

#[test]
fn contacts_and_groups_are_searchable_through_the_messaging_protocol() {
    let mut service = MessagingService::load(MemoryStateStore::default()).unwrap();
    for (id, name) in [
        ("human:owner", "Alice Owner"),
        ("human:admin", "Bob Admin"),
        ("human:member", "Carol Member"),
        ("human:restricted", "Restricted Member"),
    ] {
        service
            .handle(
                ClientEnvelope::new(
                    context(id, &format!("profile:{id}")),
                    ClientCommand::UpsertProfile {
                        actor: Actor::human(id, name),
                    },
                ),
                1,
            )
            .unwrap();
    }
    service
        .handle(
            ClientEnvelope::new(
                context("human:owner", "group"),
                ClientCommand::CreateConversation {
                    conversation: group_conversation("group:m5-search"),
                },
            ),
            2,
        )
        .unwrap();

    let contacts = service
        .handle(
            ClientEnvelope::new(
                context("human:owner", "search-contacts"),
                ClientCommand::Search {
                    query: SearchQuery {
                        text: "alice".into(),
                        scope: SearchScope::Contacts,
                        conversation_id: None,
                        sender_id: None,
                        from_ms: None,
                        to_ms: None,
                        limit: 10,
                    },
                },
            ),
            3,
        )
        .unwrap();
    assert!(matches!(
        &contacts[0].event,
        ServerEvent::SearchResults { results, .. }
            if results.iter().any(|result| result.id == "human:owner")
    ));

    let groups = service
        .handle(
            ClientEnvelope::new(
                context("human:owner", "search-groups"),
                ClientCommand::Search {
                    query: SearchQuery {
                        text: "dharma".into(),
                        scope: SearchScope::Groups,
                        conversation_id: None,
                        sender_id: None,
                        from_ms: None,
                        to_ms: None,
                        limit: 10,
                    },
                },
            ),
            4,
        )
        .unwrap();
    assert!(matches!(
        &groups[0].event,
        ServerEvent::SearchResults { results, .. }
            if results.iter().any(|result| result.id == "group:m5-search")
    ));
}

#[test]
fn admin_rights_are_operation_specific() {
    let mut engine = seed_engine("group:m5-admin");

    engine
        .execute(Command::CreateInviteLink {
            actor_id: ActorId::new("human:admin"),
            invite: InviteLink {
                id: "invite:m5".into(),
                conversation_id: ConversationId::new("group:m5-admin"),
                creator_id: ActorId::new("human:admin"),
                token: "m5-token".into(),
                name: Some("M5".into()),
                created_at_ms: 2,
                expires_at_ms: None,
                member_limit: None,
                join_request: false,
                revoked: false,
                joined_count: 0,
            },
        })
        .unwrap();

    let topic_denied = engine
        .execute(Command::UpsertForumTopic {
            actor_id: ActorId::new("human:admin"),
            topic: ForumTopicState {
                id: "topic:m5-denied".into(),
                conversation_id: ConversationId::new("group:m5-admin"),
                title: "Denied".into(),
                icon: None,
                creator_id: ActorId::new("human:admin"),
                created_at_ms: 3,
                pinned: false,
                closed: false,
                hidden: false,
                unread_count: 0,
                last_message_id: None,
            },
        })
        .unwrap_err();
    assert_eq!(topic_denied, EngineError::CommunityPermissionDenied);

    let member_denied = engine
        .execute(Command::CreateInviteLink {
            actor_id: ActorId::new("human:member"),
            invite: InviteLink {
                id: "invite:member".into(),
                conversation_id: ConversationId::new("group:m5-admin"),
                creator_id: ActorId::new("human:member"),
                token: "member-token".into(),
                name: None,
                created_at_ms: 4,
                expires_at_ms: None,
                member_limit: None,
                join_request: false,
                revoked: false,
                joined_count: 0,
            },
        })
        .unwrap_err();
    assert_eq!(member_denied, EngineError::CommunityPermissionDenied);

    let mut promoted = engine.state().communities[&ConversationId::new("group:m5-admin")].members
        [&ActorId::new("human:admin")]
        .clone();
    promoted.admin_rights.manage_topics = true;
    engine
        .execute(Command::SetCommunityMember {
            actor_id: ActorId::new("human:owner"),
            conversation_id: ConversationId::new("group:m5-admin"),
            member: promoted,
        })
        .unwrap();
    engine
        .execute(Command::UpsertForumTopic {
            actor_id: ActorId::new("human:admin"),
            topic: ForumTopicState {
                id: "topic:m5-allowed".into(),
                conversation_id: ConversationId::new("group:m5-admin"),
                title: "Allowed".into(),
                icon: None,
                creator_id: ActorId::new("human:admin"),
                created_at_ms: 5,
                pinned: false,
                closed: false,
                hidden: false,
                unread_count: 0,
                last_message_id: None,
            },
        })
        .unwrap();
}

#[test]
fn restricted_members_cannot_bypass_group_send_restrictions() {
    let mut engine = seed_engine("group:m5-restricted");
    let error = engine
        .execute(Command::QueueMessage {
            conversation_id: ConversationId::new("group:m5-restricted"),
            local_message_id: MessageId::new("message:restricted"),
            client_message_id: ClientMessageId("client:restricted".into()),
            sender_id: ActorId::new("human:restricted"),
            content: MessageContent::Text {
                text: FormattedText::plain("blocked"),
            },
            reply_to_message_id: None,
            thread_root_message_id: None,
            created_at_ms: 2,
            scheduled_at_ms: None,
            silent: false,
            protected_content: false,
        })
        .unwrap_err();
    assert!(matches!(
        error,
        EngineError::CommunitySendRestricted(id) if id == ConversationId::new("group:m5-restricted")
    ));
}

#[test]
fn group_unread_is_actor_scoped_and_clears_only_for_reader() {
    let mut service = MessagingService::load(MemoryStateStore::default()).unwrap();
    for (id, name) in [
        ("human:owner", "Owner"),
        ("human:admin", "Admin"),
        ("human:member", "Member"),
        ("human:restricted", "Restricted"),
    ] {
        service
            .handle(
                ClientEnvelope::new(
                    context(id, &format!("profile:{id}")),
                    ClientCommand::UpsertProfile {
                        actor: Actor::human(id, name),
                    },
                ),
                1,
            )
            .unwrap();
    }
    service
        .handle(
            ClientEnvelope::new(
                context("human:owner", "group"),
                ClientCommand::CreateConversation {
                    conversation: group_conversation("group:m5-unread"),
                },
            ),
            2,
        )
        .unwrap();

    let sent = service
        .handle(
            ClientEnvelope::new(
                context("human:admin", "send"),
                ClientCommand::SendMessage {
                    conversation_id: ConversationId::new("group:m5-unread"),
                    client_message_id: ClientMessageId("client:m5-unread".into()),
                    content: MessageContent::Text {
                        text: FormattedText::plain("group unread"),
                    },
                    reply_to_message_id: None,
                    thread_root_message_id: None,
                    scheduled_at_ms: None,
                    silent: false,
                    protected_content: false,
                },
            ),
            3,
        )
        .unwrap();
    let message_id = sent
        .iter()
        .find_map(|event| match &event.event {
            ServerEvent::MessageAdded { message } => Some(message.id.clone()),
            _ => None,
        })
        .unwrap();

    let owner_sync = service
        .handle(
            ClientEnvelope::new(
                context("human:owner", "sync-owner"),
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            4,
        )
        .unwrap();
    assert!(owner_sync.iter().any(|event| matches!(
        &event.event,
        ServerEvent::SyncBatch { conversations, .. }
            if conversations.iter().any(|conversation| conversation.id == ConversationId::new("group:m5-unread") && conversation.unread_count == 1)
    )));

    service
        .handle(
            ClientEnvelope::new(
                context("human:owner", "read-owner"),
                ClientCommand::MarkRead {
                    conversation_id: ConversationId::new("group:m5-unread"),
                    message_id,
                },
            ),
            5,
        )
        .unwrap();
    let owner_after = service
        .handle(
            ClientEnvelope::new(
                context("human:owner", "sync-owner-after"),
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            6,
        )
        .unwrap();
    assert!(owner_after.iter().any(|event| matches!(
        &event.event,
        ServerEvent::SyncBatch { conversations, .. }
            if conversations.iter().any(|conversation| conversation.id == ConversationId::new("group:m5-unread") && conversation.unread_count == 0)
    )));

    let member_sync = service
        .handle(
            ClientEnvelope::new(
                context("human:member", "sync-member"),
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            7,
        )
        .unwrap();
    assert!(member_sync.iter().any(|event| matches!(
        &event.event,
        ServerEvent::SyncBatch { conversations, .. }
            if conversations.iter().any(|conversation| conversation.id == ConversationId::new("group:m5-unread") && conversation.unread_count == 1)
    )));
}
