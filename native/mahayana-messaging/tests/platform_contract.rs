use fabushi_messaging_core::*;
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn bot_registry_validates_commands_and_tracks_execution() {
    let bot_id = ActorId::new("bot:research");
    let mut registry = BotRegistry::default();
    registry
        .register(BotProfile {
            actor_id: bot_id.clone(),
            description: "Research assistant".into(),
            about: "Searches and summarizes".into(),
            commands: vec![BotCommand {
                command: "research".into(),
                description: "Start research".into(),
                scopes: BTreeSet::from(["private".into(), "group".into()]),
            }],
            inline_mode_enabled: true,
            inline_placeholder: Some("Search…".into()),
            groups_allowed: true,
            privacy_mode: false,
            mini_app_id: None,
            payment_provider_ids: Vec::new(),
            business_mode: false,
        })
        .unwrap();
    let invocation = BotInvocation {
        id: "invocation:1".into(),
        bot_id,
        sender_id: ActorId::new("human:1"),
        conversation_id: ConversationId::new("chat:1"),
        command: Some("research".into()),
        text: FormattedText::plain("latest Rust news"),
        reply_to_message_id: None,
        metadata: BTreeMap::new(),
        created_at_ms: 1,
    };
    let execution = registry.begin_execution(&invocation, 2).unwrap();
    assert_eq!(execution.state, BotExecutionState::Running);
    registry
        .finish_execution(&execution.id, true, 3, None)
        .unwrap();
    assert_eq!(
        registry.executions[&execution.id].state,
        BotExecutionState::Completed
    );
}

#[test]
fn device_registry_tracks_monotonic_sync_and_revocation() {
    let actor_id = ActorId::new("human:1");
    let mut registry = DeviceRegistry::default();
    registry
        .register(DeviceSession {
            id: "session:mac".into(),
            actor_id: actor_id.clone(),
            device_id: "device:mac".into(),
            platform: DevicePlatform::Macos,
            device_name: "Mac".into(),
            app_version: "1.0.0".into(),
            created_at_ms: 1,
            last_active_at_ms: 1,
            revoked_at_ms: None,
            push_token: None,
            sync_cursor: 0,
            trusted: true,
        })
        .unwrap();
    registry.acknowledge("session:mac", 10, 2).unwrap();
    registry.acknowledge("session:mac", 8, 3).unwrap();
    assert_eq!(registry.sessions["session:mac"].sync_cursor, 10);
    assert_eq!(registry.minimum_active_cursor(&actor_id), Some(10));
    registry.revoke("session:mac", 4).unwrap();
    assert!(registry.active_for_actor(&actor_id).is_empty());
}

#[test]
fn search_indexes_humans_bots_and_channels_without_transport_assumptions() {
    let mut index = SearchIndex::default();
    let mut human = Actor::human("human:1", "善友");
    human.username = Some("shanyou".into());
    index.index_actor(human);
    index.index_actor(Actor::bot("bot:research", "Research Bot"));

    let channel = Conversation {
        id: ConversationId::new("channel:rust"),
        kind: ConversationKind::Channel,
        title: "Rust 频道".into(),
        description: Some("系统语言与异步运行时".into()),
        avatar_url: None,
        participants: Vec::new(),
        owner_id: None,
        last_message_id: None,
        last_read_message_id: None,
        unread_count: 0,
        mention_count: 0,
        pinned_message_ids: Vec::new(),
        notification_settings: NotificationSettings::default(),
        permissions: ConversationPermissions::default(),
        history_visibility: HistoryVisibility::AllMembers,
        topics: Vec::new(),
        folder_ids: Vec::new(),
        archived: false,
        pinned: false,
        marked_unread: false,
        created_at_ms: 1,
        updated_at_ms: 2,
    };
    index.index_conversation(channel);

    let contact_results = index.search(&SearchQuery {
        text: "善友".into(),
        scope: SearchScope::Contacts,
        conversation_id: None,
        sender_id: None,
        from_ms: None,
        to_ms: None,
        limit: 10,
    });
    assert_eq!(contact_results.len(), 1);
    assert_eq!(contact_results[0].id, "human:1");

    let channel_results = index.search(&SearchQuery {
        text: "Rust".into(),
        scope: SearchScope::Channels,
        conversation_id: None,
        sender_id: None,
        from_ms: None,
        to_ms: None,
        limit: 10,
    });
    assert_eq!(channel_results.len(), 1);
    assert_eq!(channel_results[0].id, "channel:rust");
}

#[test]
fn privacy_and_quiet_hours_enforce_expected_boundaries() {
    let stranger = ActorId::new("human:stranger");
    let mut privacy = PrivacySettings {
        require_mutual_contact_for_messages: true,
        ..PrivacySettings::default()
    };
    assert!(!privacy.can_receive_message_from(&stranger, false, false));
    privacy.block(BlockEntry {
        actor_id: stranger.clone(),
        blocked_at_ms: 1,
        reason: Some("spam".into()),
        report_spam: true,
    });
    assert!(privacy.is_blocked(&stranger));

    let current = ActorId::new("human:me");
    let policy = NotificationPolicy {
        quiet_hours: Some(QuietHours {
            enabled: true,
            start_minute_local: 22 * 60,
            end_minute_local: 7 * 60,
            allow_mentions: true,
            allow_calls: true,
        }),
        conversation_rules: BTreeMap::new(),
    };
    let quiet = policy.decide(
        &NotificationCandidate {
            id: "notification:1".into(),
            conversation_id: ConversationId::new("chat:1"),
            sender_id: stranger.clone(),
            title: "New message".into(),
            body: "hello".into(),
            created_at_ms: 2,
            mentioned_actor_ids: BTreeSet::new(),
            is_call: false,
            silent_message: false,
        },
        &current,
        2,
        23 * 60,
    );
    assert!(!quiet.deliver);

    let mention = policy.decide(
        &NotificationCandidate {
            id: "notification:2".into(),
            conversation_id: ConversationId::new("chat:1"),
            sender_id: stranger,
            title: "Mention".into(),
            body: "@me".into(),
            created_at_ms: 3,
            mentioned_actor_ids: BTreeSet::from([current.clone()]),
            is_call: false,
            silent_message: false,
        },
        &current,
        3,
        23 * 60,
    );
    assert!(mention.deliver);
}
