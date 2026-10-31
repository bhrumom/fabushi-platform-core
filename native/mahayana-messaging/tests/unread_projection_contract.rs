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

fn handle(
    service: &mut MessagingService<MemoryStateStore>,
    actor_id: &str,
    request_id: &str,
    command: ClientCommand,
    now_ms: i64,
) -> Vec<ServerEnvelope> {
    service
        .handle(
            ClientEnvelope::new(context(actor_id, request_id), command),
            now_ms,
        )
        .unwrap()
}

fn synced_conversation(
    service: &mut MessagingService<MemoryStateStore>,
    actor_id: &str,
    conversation_id: &str,
    now_ms: i64,
) -> Conversation {
    handle(
        service,
        actor_id,
        "sync",
        ClientCommand::Sync {
            cursor: None,
            limit: 100,
        },
        now_ms,
    )
    .into_iter()
    .find_map(|envelope| match envelope.event {
        ServerEvent::SyncBatch { conversations, .. } => conversations
            .into_iter()
            .find(|conversation| conversation.id == ConversationId::new(conversation_id)),
        _ => None,
    })
    .expect("conversation should be visible in actor sync")
}

#[test]
fn unread_projection_is_actor_scoped_and_read_cursor_survives_reload() {
    let mut service = MessagingService::load(MemoryStateStore::default()).unwrap();
    let me = ActorId::new("human:me");
    let peer = ActorId::new("human:peer");
    let conversation_id = "direct:unread-contract";

    handle(
        &mut service,
        "human:me",
        "profile-me",
        ClientCommand::UpsertProfile {
            actor: Actor::human("human:me", "Me"),
        },
        1,
    );
    handle(
        &mut service,
        "human:peer",
        "profile-peer",
        ClientCommand::UpsertProfile {
            actor: Actor::human("human:peer", "Peer"),
        },
        2,
    );
    handle(
        &mut service,
        "human:me",
        "conversation",
        ClientCommand::CreateConversation {
            conversation: Conversation::direct(
                conversation_id,
                "Unread contract",
                vec![
                    Participant {
                        actor_id: me.clone(),
                        role: ParticipantRole::Owner,
                        joined_at_ms: 3,
                        muted_until_ms: None,
                    },
                    Participant {
                        actor_id: peer.clone(),
                        role: ParticipantRole::Member,
                        joined_at_ms: 3,
                        muted_until_ms: None,
                    },
                ],
                3,
            ),
        },
        3,
    );

    let incoming = handle(
        &mut service,
        "human:peer",
        "send-1",
        ClientCommand::SendMessage {
            conversation_id: ConversationId::new(conversation_id),
            client_message_id: ClientMessageId("client:unread:1".into()),
            content: MessageContent::Text {
                text: FormattedText::plain("first unread"),
            },
            reply_to_message_id: None,
            thread_root_message_id: None,
            scheduled_at_ms: None,
            silent: false,
            protected_content: false,
        },
        4,
    );
    let first_message_id = incoming
        .iter()
        .find_map(|envelope| match &envelope.event {
            ServerEvent::MessageAdded { message } => Some(message.id.clone()),
            _ => None,
        })
        .expect("send should emit MessageAdded");

    let my_projection = synced_conversation(&mut service, "human:me", conversation_id, 5);
    assert_eq!(my_projection.unread_count, 1);
    assert_eq!(my_projection.last_read_message_id, None);

    let peer_projection = synced_conversation(&mut service, "human:peer", conversation_id, 6);
    assert_eq!(peer_projection.unread_count, 0);

    handle(
        &mut service,
        "human:me",
        "read-1",
        ClientCommand::MarkRead {
            conversation_id: ConversationId::new(conversation_id),
            message_id: first_message_id.clone(),
        },
        7,
    );
    let my_projection = synced_conversation(&mut service, "human:me", conversation_id, 8);
    assert_eq!(my_projection.unread_count, 0);
    assert_eq!(
        my_projection.last_read_message_id.as_deref(),
        Some(first_message_id.0.as_str())
    );

    let store = service.into_store();
    let mut restored = MessagingService::load(store).unwrap();
    let restored_projection = synced_conversation(&mut restored, "human:me", conversation_id, 9);
    assert_eq!(restored_projection.unread_count, 0);
    assert_eq!(
        restored_projection.last_read_message_id.as_deref(),
        Some(first_message_id.0.as_str())
    );
}

#[test]
fn draft_and_marked_unread_are_actor_scoped_and_persisted() {
    let mut service = MessagingService::load(MemoryStateStore::default()).unwrap();
    let me = ActorId::new("human:draft-me");
    let peer = ActorId::new("human:draft-peer");
    let conversation_id = "direct:draft-unread-contract";

    handle(
        &mut service,
        "human:draft-me",
        "profile-me",
        ClientCommand::UpsertProfile {
            actor: Actor::human("human:draft-me", "Me"),
        },
        1,
    );
    handle(
        &mut service,
        "human:draft-peer",
        "profile-peer",
        ClientCommand::UpsertProfile {
            actor: Actor::human("human:draft-peer", "Peer"),
        },
        2,
    );
    handle(
        &mut service,
        "human:draft-me",
        "conversation",
        ClientCommand::CreateConversation {
            conversation: Conversation::direct(
                conversation_id,
                "Draft contract",
                vec![
                    Participant {
                        actor_id: me.clone(),
                        role: ParticipantRole::Owner,
                        joined_at_ms: 3,
                        muted_until_ms: None,
                    },
                    Participant {
                        actor_id: peer.clone(),
                        role: ParticipantRole::Member,
                        joined_at_ms: 3,
                        muted_until_ms: None,
                    },
                ],
                3,
            ),
        },
        3,
    );

    handle(
        &mut service,
        "human:draft-me",
        "draft",
        ClientCommand::SetDraft {
            conversation_id: ConversationId::new(conversation_id),
            text: "跨设备草稿".into(),
            reply_to_message_id: None,
        },
        4,
    );
    handle(
        &mut service,
        "human:draft-me",
        "marked-unread",
        ClientCommand::SetMarkedUnread {
            conversation_id: ConversationId::new(conversation_id),
            marked_unread: true,
        },
        5,
    );

    let mine = handle(
        &mut service,
        "human:draft-me",
        "sync-me",
        ClientCommand::Sync {
            cursor: None,
            limit: 100,
        },
        6,
    );
    let (my_conversation, my_drafts) = mine
        .into_iter()
        .find_map(|envelope| match envelope.event {
            ServerEvent::SyncBatch {
                conversations,
                drafts,
                ..
            } => Some((
                conversations
                    .into_iter()
                    .find(|item| item.id == ConversationId::new(conversation_id))
                    .unwrap(),
                drafts,
            )),
            _ => None,
        })
        .unwrap();
    assert!(my_conversation.marked_unread);
    assert_eq!(my_drafts.len(), 1);
    assert_eq!(my_drafts[0].text, "跨设备草稿");
    assert_eq!(my_drafts[0].actor_id, me);

    let theirs = handle(
        &mut service,
        "human:draft-peer",
        "sync-peer",
        ClientCommand::Sync {
            cursor: None,
            limit: 100,
        },
        7,
    );
    let (peer_conversation, peer_drafts) = theirs
        .into_iter()
        .find_map(|envelope| match envelope.event {
            ServerEvent::SyncBatch {
                conversations,
                drafts,
                ..
            } => Some((
                conversations
                    .into_iter()
                    .find(|item| item.id == ConversationId::new(conversation_id))
                    .unwrap(),
                drafts,
            )),
            _ => None,
        })
        .unwrap();
    assert!(!peer_conversation.marked_unread);
    assert!(peer_drafts.is_empty());

    let store = service.into_store();
    let mut restored = MessagingService::load(store).unwrap();
    let restored_sync = handle(
        &mut restored,
        "human:draft-me",
        "sync-restored",
        ClientCommand::Sync {
            cursor: None,
            limit: 100,
        },
        8,
    );
    let restored_drafts = restored_sync
        .into_iter()
        .find_map(|envelope| match envelope.event {
            ServerEvent::SyncBatch { drafts, .. } => Some(drafts),
            _ => None,
        })
        .unwrap();
    assert_eq!(restored_drafts.len(), 1);
    assert_eq!(restored_drafts[0].text, "跨设备草稿");
}

#[test]
fn poll_votes_share_counts_but_keep_actor_selection_private() {
    let mut service = MessagingService::load(MemoryStateStore::default()).unwrap();
    let me = ActorId::new("human:poll-me");
    let peer = ActorId::new("human:poll-peer");
    let conversation_id = "direct:poll-contract";

    handle(
        &mut service,
        "human:poll-me",
        "profile-me",
        ClientCommand::UpsertProfile {
            actor: Actor::human("human:poll-me", "Me"),
        },
        1,
    );
    handle(
        &mut service,
        "human:poll-peer",
        "profile-peer",
        ClientCommand::UpsertProfile {
            actor: Actor::human("human:poll-peer", "Peer"),
        },
        2,
    );
    handle(
        &mut service,
        "human:poll-me",
        "conversation",
        ClientCommand::CreateConversation {
            conversation: Conversation::direct(
                conversation_id,
                "Poll contract",
                vec![
                    Participant {
                        actor_id: me.clone(),
                        role: ParticipantRole::Owner,
                        joined_at_ms: 3,
                        muted_until_ms: None,
                    },
                    Participant {
                        actor_id: peer.clone(),
                        role: ParticipantRole::Member,
                        joined_at_ms: 3,
                        muted_until_ms: None,
                    },
                ],
                3,
            ),
        },
        3,
    );

    let sent = handle(
        &mut service,
        "human:poll-peer",
        "send-poll",
        ClientCommand::SendMessage {
            conversation_id: ConversationId::new(conversation_id),
            client_message_id: ClientMessageId("client:poll:1".into()),
            content: MessageContent::Poll {
                question: FormattedText::plain("选一个"),
                options: vec![
                    PollOption {
                        id: "a".into(),
                        text: "A".into(),
                        voter_count: 0,
                        chosen: false,
                        correct: None,
                    },
                    PollOption {
                        id: "b".into(),
                        text: "B".into(),
                        voter_count: 0,
                        chosen: false,
                        correct: None,
                    },
                ],
                anonymous: true,
                multiple_answers: false,
                quiz: false,
            },
            reply_to_message_id: None,
            thread_root_message_id: None,
            scheduled_at_ms: None,
            silent: false,
            protected_content: false,
        },
        4,
    );
    let message_id = sent
        .iter()
        .find_map(|envelope| match &envelope.event {
            ServerEvent::MessageAdded { message } => Some(message.id.clone()),
            _ => None,
        })
        .unwrap();

    handle(
        &mut service,
        "human:poll-me",
        "vote",
        ClientCommand::VotePoll {
            conversation_id: ConversationId::new(conversation_id),
            message_id: message_id.clone(),
            option_ids: vec!["a".into()],
        },
        5,
    );

    let my_sync = handle(
        &mut service,
        "human:poll-me",
        "sync-me",
        ClientCommand::Sync {
            cursor: None,
            limit: 100,
        },
        6,
    );
    let my_poll = my_sync
        .into_iter()
        .find_map(|envelope| match envelope.event {
            ServerEvent::SyncBatch { messages, .. } => messages
                .into_iter()
                .find(|message| message.id == message_id),
            _ => None,
        })
        .unwrap();
    let MessageContent::Poll {
        options: my_options,
        ..
    } = my_poll.content
    else {
        panic!("expected poll")
    };
    assert_eq!(my_options[0].voter_count, 1);
    assert!(my_options[0].chosen);
    assert!(!my_options[1].chosen);

    let peer_sync = handle(
        &mut service,
        "human:poll-peer",
        "sync-peer",
        ClientCommand::Sync {
            cursor: None,
            limit: 100,
        },
        7,
    );
    let peer_poll = peer_sync
        .into_iter()
        .find_map(|envelope| match envelope.event {
            ServerEvent::SyncBatch { messages, .. } => messages
                .into_iter()
                .find(|message| message.id == message_id),
            _ => None,
        })
        .unwrap();
    let MessageContent::Poll {
        options: peer_options,
        ..
    } = peer_poll.content
    else {
        panic!("expected poll")
    };
    assert_eq!(peer_options[0].voter_count, 1);
    assert!(!peer_options[0].chosen);
    assert!(!peer_options[1].chosen);

    let invalid = service.handle(
        ClientEnvelope::new(
            context("human:poll-me", "invalid-vote"),
            ClientCommand::VotePoll {
                conversation_id: ConversationId::new(conversation_id),
                message_id,
                option_ids: vec!["a".into(), "b".into()],
            },
        ),
        8,
    );
    assert!(matches!(
        invalid,
        Err(MessagingServiceError::Engine(EngineError::InvalidPollVote))
    ));
}

#[test]
fn conversation_management_enforces_owner_admin_boundaries_and_removal() {
    let mut service = MessagingService::load(MemoryStateStore::default()).unwrap();
    for (id, name) in [
        ("human:owner", "Owner"),
        ("human:admin", "Admin"),
        ("human:member", "Member"),
        ("human:new", "New"),
    ] {
        handle(
            &mut service,
            id,
            &format!("profile-{id}"),
            ClientCommand::UpsertProfile {
                actor: Actor::human(id, name),
            },
            1,
        );
    }

    let conversation_id = "group:management-contract";
    let mut conversation = Conversation::direct(
        conversation_id,
        "Managed group",
        vec![
            Participant {
                actor_id: ActorId::new("human:owner"),
                role: ParticipantRole::Owner,
                joined_at_ms: 2,
                muted_until_ms: None,
            },
            Participant {
                actor_id: ActorId::new("human:admin"),
                role: ParticipantRole::Admin,
                joined_at_ms: 2,
                muted_until_ms: None,
            },
            Participant {
                actor_id: ActorId::new("human:member"),
                role: ParticipantRole::Member,
                joined_at_ms: 2,
                muted_until_ms: None,
            },
        ],
        2,
    );
    conversation.kind = ConversationKind::Group;
    conversation.owner_id = Some(ActorId::new("human:owner"));
    handle(
        &mut service,
        "human:owner",
        "create-group",
        ClientCommand::CreateConversation { conversation },
        2,
    );

    let mut community = CommunityState::new(ConversationId::new(conversation_id));
    community.upsert_member(CommunityMember {
        actor_id: ActorId::new("human:admin"),
        status: MemberStatus::Administrator,
        admin_title: None,
        admin_rights: AdminRights {
            add_admins: true,
            ..AdminRights::default()
        },
        restrictions: MemberRestrictions::default(),
        joined_at_ms: 2,
        invited_by: Some(ActorId::new("human:owner")),
    });
    handle(
        &mut service,
        "human:owner",
        "create-community",
        ClientCommand::UpdateCommunity { community },
        2,
    );

    handle(
        &mut service,
        "human:admin",
        "admin-add-member",
        ClientCommand::SetConversationParticipant {
            conversation_id: ConversationId::new(conversation_id),
            participant: Participant {
                actor_id: ActorId::new("human:new"),
                role: ParticipantRole::Member,
                joined_at_ms: 3,
                muted_until_ms: None,
            },
        },
        3,
    );
    assert!(
        synced_conversation(&mut service, "human:new", conversation_id, 4)
            .participants
            .iter()
            .any(|participant| participant.actor_id == ActorId::new("human:new"))
    );

    let member_edit = service.handle(
        ClientEnvelope::new(
            context("human:member", "member-edit"),
            ClientCommand::UpdateConversationInfo {
                conversation_id: ConversationId::new(conversation_id),
                title: "Should fail".into(),
                description: None,
            },
        ),
        5,
    );
    assert!(matches!(
        member_edit,
        Err(MessagingServiceError::UnauthorizedCommand(_))
    ));

    handle(
        &mut service,
        "human:owner",
        "owner-promote",
        ClientCommand::SetConversationParticipant {
            conversation_id: ConversationId::new(conversation_id),
            participant: Participant {
                actor_id: ActorId::new("human:new"),
                role: ParticipantRole::Admin,
                joined_at_ms: 3,
                muted_until_ms: None,
            },
        },
        6,
    );

    let admin_remove_admin = service.handle(
        ClientEnvelope::new(
            context("human:admin", "admin-remove-admin"),
            ClientCommand::RemoveConversationParticipant {
                conversation_id: ConversationId::new(conversation_id),
                actor_id: ActorId::new("human:new"),
            },
        ),
        7,
    );
    assert!(matches!(
        admin_remove_admin,
        Err(MessagingServiceError::UnauthorizedCommand(_))
    ));

    handle(
        &mut service,
        "human:owner",
        "owner-remove-member",
        ClientCommand::RemoveConversationParticipant {
            conversation_id: ConversationId::new(conversation_id),
            actor_id: ActorId::new("human:member"),
        },
        8,
    );
    let removed_sync = handle(
        &mut service,
        "human:member",
        "removed-sync",
        ClientCommand::Sync {
            cursor: None,
            limit: 100,
        },
        9,
    );
    let still_visible = removed_sync
        .into_iter()
        .any(|envelope| match envelope.event {
            ServerEvent::SyncBatch { conversations, .. } => conversations
                .into_iter()
                .any(|conversation| conversation.id == ConversationId::new(conversation_id)),
            _ => false,
        });
    assert!(!still_visible);
}
