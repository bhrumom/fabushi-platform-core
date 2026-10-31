use fabushi_messaging_core::*;

fn participant(actor_id: &str, role: ParticipantRole) -> Participant {
    Participant {
        actor_id: ActorId::new(actor_id),
        role,
        joined_at_ms: 1,
        muted_until_ms: None,
    }
}

#[test]
fn humans_and_bots_share_the_same_conversation_engine() {
    let mut engine = MessagingEngine::new();
    engine
        .execute(Command::UpsertActor {
            actor: Actor::human("human:1", "善友"),
        })
        .unwrap();
    engine
        .execute(Command::UpsertActor {
            actor: Actor::assistant("agent:mahayana", "大乘助手"),
        })
        .unwrap();
    engine
        .execute(Command::UpsertActor {
            actor: Actor::bot("bot:publisher", "发布机器人"),
        })
        .unwrap();

    let conversation = Conversation::direct(
        "chat:unified",
        "善友 · 大乘助手 · 发布机器人",
        vec![
            participant("human:1", ParticipantRole::Owner),
            participant("agent:mahayana", ParticipantRole::Member),
            participant("bot:publisher", ParticipantRole::Member),
        ],
        10,
    );
    engine
        .execute(Command::UpsertConversation { conversation })
        .unwrap();
    let events = engine
        .execute(Command::QueueMessage {
            conversation_id: ConversationId::new("chat:unified"),
            local_message_id: MessageId::new("local:1"),
            client_message_id: ClientMessageId("client:1".into()),
            sender_id: ActorId::new("human:1"),
            content: MessageContent::Text {
                text: FormattedText::plain("请两个机器人协作处理"),
            },
            reply_to_message_id: None,
            thread_root_message_id: None,
            created_at_ms: 20,
            scheduled_at_ms: None,
            silent: false,
            protected_content: false,
        })
        .unwrap();
    assert!(matches!(events.as_slice(), [Event::MessageQueued { .. }]));
    assert_eq!(engine.state().actors.len(), 3);
    assert_eq!(
        engine.state().messages[&ConversationId::new("chat:unified")].len(),
        1
    );
}

#[test]
fn invoice_and_paid_order_use_the_same_chat_domain() {
    let mut engine = MessagingEngine::new();
    engine
        .execute(Command::UpsertActor {
            actor: Actor::human("buyer", "购买者"),
        })
        .unwrap();
    engine
        .execute(Command::UpsertActor {
            actor: Actor::bot("seller", "商店机器人"),
        })
        .unwrap();
    engine
        .execute(Command::UpsertConversation {
            conversation: Conversation::direct(
                "chat:shop",
                "商店",
                vec![
                    participant("buyer", ParticipantRole::Owner),
                    participant("seller", ParticipantRole::Member),
                ],
                1,
            ),
        })
        .unwrap();
    let invoice = Invoice {
        id: "invoice:1".into(),
        conversation_id: ConversationId::new("chat:shop"),
        seller_id: ActorId::new("seller"),
        title: "Mini App Pro".into(),
        description: "数字商品".into(),
        kind: InvoiceKind::DigitalGoods,
        currency: "USD".into(),
        prices: vec![PriceLine {
            label: "Pro".into(),
            amount: Money::new("USD", 999),
        }],
        payload: "product=pro".into(),
        provider_id: "fabushi-pay".into(),
        start_parameter: None,
        request_name: false,
        request_email: true,
        request_phone: false,
        request_shipping_address: false,
        flexible_shipping: false,
        created_at_ms: 2,
        expires_at_ms: None,
    };
    engine.execute(Command::CreateInvoice { invoice }).unwrap();
    engine
        .execute(Command::UpsertOrder {
            order: PaymentOrder {
                id: "order:1".into(),
                invoice_id: "invoice:1".into(),
                buyer_id: ActorId::new("buyer"),
                status: PaymentStatus::Pending,
                amount: Money::new("USD", 999),
                customer: None,
                provider_payment_id: None,
                provider_receipt_url: None,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
        })
        .unwrap();
    engine.complete_paid_order("order:1", 4).unwrap();
    assert_eq!(engine.state().orders["order:1"].status, PaymentStatus::Paid);
}

#[test]
fn mini_app_permissions_are_enforced_before_host_calls() {
    let mut engine = MessagingEngine::new();
    engine
        .execute(Command::UpsertActor {
            actor: Actor::human("human:1", "善友"),
        })
        .unwrap();
    let manifest = MiniAppManifest {
        id: "mini:shop".into(),
        name: "商店".into(),
        version: "1.0.0".into(),
        description: "支付测试".into(),
        icon_url: None,
        start_url: "https://mini.example.invalid".into(),
        allowed_origins: vec!["https://mini.example.invalid".into()],
        requested_permissions: vec![MiniAppPermission::Payments],
        bot_actor_id: None,
        verified: true,
    };
    engine
        .execute(Command::InstallMiniApp { manifest })
        .unwrap();
    engine
        .execute(Command::GrantMiniApp {
            grant: MiniAppGrant {
                mini_app_id: "mini:shop".into(),
                actor_id: ActorId::new("human:1"),
                permissions: vec![MiniAppPermission::Payments],
                granted_at_ms: 1,
                expires_at_ms: None,
            },
        })
        .unwrap();
    engine
        .execute(Command::OpenMiniApp {
            session: MiniAppSession {
                id: "session:1".into(),
                mini_app_id: "mini:shop".into(),
                actor_id: ActorId::new("human:1"),
                conversation_id: None,
                start_parameter: None,
                granted_permissions: vec![MiniAppPermission::Payments],
                opened_at_ms: 2,
                expires_at_ms: 20,
            },
        })
        .unwrap();
    let error = engine
        .execute(Command::MiniAppCall {
            session_id: "session:1".into(),
            request_id: "request:1".into(),
            request: MiniAppRequest::RequestLocation,
        })
        .unwrap_err();
    assert_eq!(
        error,
        EngineError::MiniAppPermissionDenied(MiniAppPermission::Location)
    );
}

#[test]
fn wire_protocol_is_fabushi_owned_and_versioned() {
    let envelope = ClientEnvelope::new(
        RequestContext {
            request_id: "req:1".into(),
            device_id: "desktop:1".into(),
            actor_id: ActorId::new("human:1"),
            session_id: "session:1".into(),
            sent_at_ms: 10,
        },
        ClientCommand::Sync {
            cursor: None,
            limit: 100,
        },
    );
    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.contains("protocolVersion"));
    assert!(!json.to_ascii_lowercase().contains("mtproto"));
    assert_eq!(
        envelope.protocol_version,
        FABUSHI_MESSAGING_PROTOCOL_VERSION
    );
}

#[test]
fn forwarding_preserves_origin_and_rejects_protected_content() {
    let mut engine = MessagingEngine::new();
    engine
        .execute(Command::UpsertActor {
            actor: Actor::human("human:forwarder", "转发者"),
        })
        .unwrap();
    for (id, title) in [("chat:source", "来源"), ("chat:destination", "目标")] {
        engine
            .execute(Command::UpsertConversation {
                conversation: Conversation::direct(
                    id,
                    title,
                    vec![participant("human:forwarder", ParticipantRole::Owner)],
                    1,
                ),
            })
            .unwrap();
    }
    engine
        .execute(Command::QueueMessage {
            conversation_id: ConversationId::new("chat:source"),
            local_message_id: MessageId::new("source:1"),
            client_message_id: ClientMessageId("client:source".into()),
            sender_id: ActorId::new("human:forwarder"),
            content: MessageContent::Text {
                text: FormattedText::plain("可转发内容"),
            },
            reply_to_message_id: None,
            thread_root_message_id: None,
            created_at_ms: 2,
            scheduled_at_ms: None,
            silent: false,
            protected_content: false,
        })
        .unwrap();
    engine
        .execute(Command::ForwardMessage {
            source_conversation_id: ConversationId::new("chat:source"),
            message_id: MessageId::new("source:1"),
            destination_conversation_id: ConversationId::new("chat:destination"),
            local_message_id: MessageId::new("forward:1"),
            client_message_id: ClientMessageId("client:forward".into()),
            sender_id: ActorId::new("human:forwarder"),
            created_at_ms: 3,
        })
        .unwrap();
    let forwarded = &engine.state().messages[&ConversationId::new("chat:destination")]
        [&MessageId::new("forward:1")];
    assert_eq!(
        forwarded.forward_origin.as_deref(),
        Some("chat:source:source:1")
    );
    assert!(matches!(forwarded.content, MessageContent::Text { .. }));

    engine
        .execute(Command::QueueMessage {
            conversation_id: ConversationId::new("chat:source"),
            local_message_id: MessageId::new("source:protected"),
            client_message_id: ClientMessageId("client:protected".into()),
            sender_id: ActorId::new("human:forwarder"),
            content: MessageContent::Text {
                text: FormattedText::plain("禁止转发"),
            },
            reply_to_message_id: None,
            thread_root_message_id: None,
            created_at_ms: 4,
            scheduled_at_ms: None,
            silent: false,
            protected_content: true,
        })
        .unwrap();
    let error = engine
        .execute(Command::ForwardMessage {
            source_conversation_id: ConversationId::new("chat:source"),
            message_id: MessageId::new("source:protected"),
            destination_conversation_id: ConversationId::new("chat:destination"),
            local_message_id: MessageId::new("forward:protected"),
            client_message_id: ClientMessageId("client:forward-protected".into()),
            sender_id: ActorId::new("human:forwarder"),
            created_at_ms: 5,
        })
        .unwrap_err();
    assert_eq!(error, EngineError::ProtectedContent);
}

#[test]
fn secret_conversations_reject_plaintext_and_protect_encrypted_messages() {
    let mut engine = MessagingEngine::new();
    for (id, name) in [("human:alice", "Alice"), ("human:bob", "Bob")] {
        engine
            .execute(Command::UpsertActor {
                actor: Actor::human(id, name),
            })
            .unwrap();
    }
    let mut conversation = Conversation::direct(
        "secret:contract",
        "Secret",
        vec![
            participant("human:alice", ParticipantRole::Owner),
            participant("human:bob", ParticipantRole::Member),
        ],
        1,
    );
    conversation.kind = ConversationKind::Secret;
    engine
        .execute(Command::UpsertConversation { conversation })
        .unwrap();

    let plaintext_error = engine
        .execute(Command::QueueMessage {
            conversation_id: ConversationId::new("secret:contract"),
            local_message_id: MessageId::new("secret:plain"),
            client_message_id: ClientMessageId("client:plain".into()),
            sender_id: ActorId::new("human:alice"),
            content: MessageContent::Text {
                text: FormattedText::plain("must never persist"),
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
        plaintext_error,
        EngineError::SecretPlaintextRejected
    ));

    let alice_private = SecretPrivateKey::generate().unwrap();
    let bob_private = SecretPrivateKey::generate().unwrap();
    let mut alice_session = SecretChatSession::establish(
        ConversationId::new("secret:contract"),
        ActorId::new("human:alice"),
        ActorId::new("human:bob"),
        &alice_private,
        &bob_private.public_key(),
        1,
    )
    .unwrap();
    let envelope = alice_session.encrypt(b"encrypted payload").unwrap();
    engine
        .execute(Command::QueueMessage {
            conversation_id: ConversationId::new("secret:contract"),
            local_message_id: MessageId::new("secret:cipher"),
            client_message_id: ClientMessageId("client:cipher".into()),
            sender_id: ActorId::new("human:alice"),
            content: MessageContent::Secret { envelope },
            reply_to_message_id: None,
            thread_root_message_id: None,
            created_at_ms: 3,
            scheduled_at_ms: None,
            silent: false,
            protected_content: false,
        })
        .unwrap();
    let stored = &engine.state().messages[&ConversationId::new("secret:contract")]
        [&MessageId::new("secret:cipher")];
    assert!(stored.protected_content);
    assert!(matches!(stored.content, MessageContent::Secret { .. }));

    let forward_error = engine
        .execute(Command::ForwardMessage {
            source_conversation_id: ConversationId::new("secret:contract"),
            message_id: MessageId::new("secret:cipher"),
            destination_conversation_id: ConversationId::new("secret:contract"),
            local_message_id: MessageId::new("secret:forward"),
            client_message_id: ClientMessageId("client:forward-secret".into()),
            sender_id: ActorId::new("human:alice"),
            created_at_ms: 4,
        })
        .unwrap_err();
    assert!(matches!(forward_error, EngineError::ProtectedContent));
}

#[test]
fn stories_communities_and_bot_execution_enforce_actor_permissions() {
    let mut engine = MessagingEngine::new();
    for actor in [
        Actor::human("human:owner", "Owner"),
        Actor::human("human:member", "Member"),
        Actor::human("human:outsider", "Outsider"),
        Actor::bot("bot:helper", "Helper"),
    ] {
        engine.execute(Command::UpsertActor { actor }).unwrap();
    }

    let mut group = Conversation::direct(
        "group:secure",
        "Secure group",
        vec![
            participant("human:owner", ParticipantRole::Owner),
            participant("human:member", ParticipantRole::Member),
            participant("bot:helper", ParticipantRole::Member),
        ],
        1,
    );
    group.kind = ConversationKind::Group;
    group.owner_id = Some(ActorId::new("human:owner"));
    engine
        .execute(Command::UpsertConversation {
            conversation: group,
        })
        .unwrap();

    let mut community = CommunityState::new(ConversationId::new("group:secure"));
    community.upsert_member(CommunityMember {
        actor_id: ActorId::new("human:owner"),
        status: MemberStatus::Owner,
        admin_title: Some("Owner".into()),
        admin_rights: AdminRights::default(),
        restrictions: MemberRestrictions::default(),
        joined_at_ms: 1,
        invited_by: None,
    });
    engine
        .execute(Command::UpdateCommunity {
            actor_id: ActorId::new("human:owner"),
            community,
        })
        .unwrap();
    let denied = engine
        .execute(Command::SetCommunityMember {
            actor_id: ActorId::new("human:member"),
            conversation_id: ConversationId::new("group:secure"),
            member: CommunityMember {
                actor_id: ActorId::new("human:outsider"),
                status: MemberStatus::Member,
                admin_title: None,
                admin_rights: AdminRights::default(),
                restrictions: MemberRestrictions::default(),
                joined_at_ms: 2,
                invited_by: Some(ActorId::new("human:member")),
            },
        })
        .unwrap_err();
    assert!(matches!(denied, EngineError::CommunityPermissionDenied));

    let story = Story {
        id: StoryId("story:1".into()),
        owner_id: ActorId::new("human:owner"),
        media: MediaRef {
            id: "blob-story-1".into(),
            file_name: Some("story.jpg".into()),
            mime_type: Some("image/jpeg".into()),
            size_bytes: Some(10),
            width: Some(1080),
            height: Some(1920),
            duration_ms: None,
            thumbnail_id: None,
            local_path: None,
            remote_url: Some("fabushi-blob://blob-story-1".into()),
            content_hash: None,
        },
        caption: FormattedText::plain("selected story"),
        privacy: StoryPrivacy {
            kind: StoryPrivacyKind::Selected,
            included_actor_ids: std::collections::BTreeSet::from([ActorId::new("human:member")]),
            excluded_actor_ids: std::collections::BTreeSet::new(),
        },
        created_at_ms: 10,
        expires_at_ms: 100,
        edited_at_ms: None,
        pinned_to_profile: false,
        protected_content: true,
        allow_replies: true,
        views: std::collections::BTreeMap::new(),
    };
    engine
        .execute(Command::PublishStory {
            actor_id: ActorId::new("human:owner"),
            story,
        })
        .unwrap();
    engine
        .execute(Command::ViewStory {
            actor_id: ActorId::new("human:member"),
            story_id: StoryId("story:1".into()),
            viewed_at_ms: 20,
        })
        .unwrap();
    let denied = engine
        .execute(Command::ViewStory {
            actor_id: ActorId::new("human:outsider"),
            story_id: StoryId("story:1".into()),
            viewed_at_ms: 21,
        })
        .unwrap_err();
    assert!(matches!(denied, EngineError::StoryPermissionDenied(_)));

    engine
        .execute(Command::RegisterBot {
            actor_id: ActorId::new("bot:helper"),
            profile: BotProfile {
                actor_id: ActorId::new("bot:helper"),
                description: "Unified helper".into(),
                about: "Runs in the same MessagingService".into(),
                commands: vec![BotCommand {
                    command: "help".into(),
                    description: "Help".into(),
                    scopes: std::collections::BTreeSet::from(["group".into()]),
                }],
                inline_mode_enabled: false,
                inline_placeholder: None,
                groups_allowed: true,
                privacy_mode: false,
                mini_app_id: None,
                payment_provider_ids: Vec::new(),
                business_mode: false,
            },
        })
        .unwrap();
    let invocation = BotInvocation {
        id: "invoke:1".into(),
        bot_id: ActorId::new("bot:helper"),
        sender_id: ActorId::new("human:member"),
        conversation_id: ConversationId::new("group:secure"),
        command: Some("help".into()),
        text: FormattedText::plain("help me"),
        reply_to_message_id: None,
        metadata: std::collections::BTreeMap::new(),
        created_at_ms: 30,
    };
    let events = engine
        .execute(Command::BeginBotInvocation {
            actor_id: ActorId::new("human:member"),
            invocation,
            created_at_ms: 31,
        })
        .unwrap();
    let execution_id = match &events[0] {
        Event::BotRegistryChanged {
            execution: Some(execution),
            ..
        } => execution.id.clone(),
        other => panic!("unexpected bot event: {other:?}"),
    };
    engine
        .execute(Command::FinishBotExecution {
            actor_id: ActorId::new("bot:helper"),
            execution_id,
            success: true,
            finished_at_ms: 32,
            error: None,
        })
        .unwrap();
}
