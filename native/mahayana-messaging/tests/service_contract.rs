use fabushi_messaging_core::*;
use std::collections::BTreeMap;

fn context(actor_id: &str) -> RequestContext {
    RequestContext {
        request_id: "request:1".into(),
        device_id: "desktop:1".into(),
        actor_id: ActorId::new(actor_id),
        session_id: "session:1".into(),
        sent_at_ms: 10,
    }
}

#[test]
fn self_hosted_service_persists_and_restores_state() {
    let store = MemoryStateStore::default();
    let mut service = MessagingService::load(store).unwrap();

    service
        .handle(
            ClientEnvelope::new(
                context("human:1"),
                ClientCommand::UpsertProfile {
                    actor: Actor::human("human:1", "善友"),
                },
            ),
            10,
        )
        .unwrap();

    service
        .handle(
            ClientEnvelope::new(
                context("human:1"),
                ClientCommand::CreateConversation {
                    conversation: Conversation::direct(
                        "chat:1",
                        "自建会话",
                        vec![Participant {
                            actor_id: ActorId::new("human:1"),
                            role: ParticipantRole::Owner,
                            joined_at_ms: 10,
                            muted_until_ms: None,
                        }],
                        10,
                    ),
                },
            ),
            11,
        )
        .unwrap();

    service
        .handle(
            ClientEnvelope::new(
                context("human:1"),
                ClientCommand::SendMessage {
                    conversation_id: ConversationId::new("chat:1"),
                    client_message_id: ClientMessageId("client:1".into()),
                    content: MessageContent::Text {
                        text: FormattedText::plain("完全自建消息"),
                    },
                    reply_to_message_id: None,
                    thread_root_message_id: None,
                    scheduled_at_ms: None,
                    silent: false,
                    protected_content: false,
                },
            ),
            12,
        )
        .unwrap();

    let cursor = service.cursor();
    assert!(cursor >= 3);
    let store = service.into_store();
    let restored = MessagingService::load(store).unwrap();
    assert_eq!(restored.cursor(), cursor);
    assert_eq!(restored.engine().state().actors.len(), 1);
    assert_eq!(restored.engine().state().conversations.len(), 1);
    assert_eq!(
        restored.engine().state().messages[&ConversationId::new("chat:1")].len(),
        1
    );
}

#[test]
fn sync_uses_the_fabushi_protocol_cursor() {
    let mut service = MessagingService::load(MemoryStateStore::default()).unwrap();
    let events = service
        .handle(
            ClientEnvelope::new(
                context("human:1"),
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            20,
        )
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].protocol_version,
        FABUSHI_MESSAGING_PROTOCOL_VERSION
    );
    assert!(matches!(events[0].event, ServerEvent::SyncBatch { .. }));
}

#[test]
fn realtime_state_tracks_group_call_media_and_typing() {
    let call_id = CallId("call:1".into());
    let actor_id = ActorId::new("human:1");
    let mut participants = BTreeMap::new();
    participants.insert(
        actor_id.clone(),
        CallParticipant {
            actor_id: actor_id.clone(),
            joined_at_ms: Some(1),
            left_at_ms: None,
            muted: false,
            video_enabled: true,
            screen_sharing: false,
            speaking: false,
        },
    );
    let mut realtime = RealtimeState::default();
    realtime
        .create_call(CallSession {
            id: call_id.clone(),
            conversation_id: ConversationId::new("group:1"),
            kind: CallKind::GroupVideo,
            state: CallState::Active,
            initiator_id: actor_id.clone(),
            participants,
            route: Some(CallRoute {
                region: "auto".into(),
                signaling_url: "wss://realtime.fabushi.invalid".into(),
                media_relay_urls: vec!["turns://relay.fabushi.invalid".into()],
                ice_servers: Vec::new(),
                end_to_end_encrypted: true,
            }),
            created_at_ms: 1,
            connected_at_ms: Some(2),
            ended_at_ms: None,
        })
        .unwrap();
    realtime
        .set_participant_media(&call_id, &actor_id, true, false, true)
        .unwrap();
    let participant = &realtime.calls[&call_id].participants[&actor_id];
    assert!(participant.muted);
    assert!(!participant.video_enabled);
    assert!(participant.screen_sharing);

    realtime.set_typing(TypingState {
        conversation_id: ConversationId::new("group:1"),
        actor_id,
        action: "typing".into(),
        expires_at_ms: 50,
    });
    realtime.expire_typing(51);
    assert!(realtime.typing.is_empty());
}

#[test]
fn messaging_service_streams_blob_chunks_to_self_hosted_storage() {
    let root = std::env::temp_dir().join(format!(
        "fabushi-messaging-blob-contract-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let blob_root = root.join("blobs");
    let store = MemoryStateStore::default();
    let mut service =
        MessagingService::load_with_blob_store(store, FileBlobStore::new(&blob_root)).unwrap();
    let id = BlobId::new("blob-contract-1").unwrap();
    let metadata = BlobMetadata {
        id: id.clone(),
        file_name: "hello.txt".into(),
        mime_type: "text/plain".into(),
        size_bytes: 5,
        content_hash: None,
        created_at_ms: 10,
    };
    service
        .handle(
            ClientEnvelope::new(
                context("human:1"),
                ClientCommand::BeginBlobUpload {
                    metadata: metadata.clone(),
                },
            ),
            10,
        )
        .unwrap();
    service
        .handle(
            ClientEnvelope::new(
                context("human:1"),
                ClientCommand::AppendBlobChunk {
                    blob_id: id.clone(),
                    offset: 0,
                    data_base64: "aGVsbG8=".into(),
                },
            ),
            11,
        )
        .unwrap();
    let events = service
        .handle(
            ClientEnvelope::new(
                context("human:1"),
                ClientCommand::FinishBlobUpload {
                    blob_id: id.clone(),
                },
            ),
            12,
        )
        .unwrap();
    assert!(matches!(events[0].event, ServerEvent::BlobReady { .. }));
    assert_eq!(
        FileBlobStore::new(&blob_root)
            .read_range(&id, 0, 5)
            .unwrap(),
        b"hello"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn wallet_checkout_and_refund_settle_invoice_atomically() {
    let store = MemoryStateStore::default();
    let mut service = MessagingService::load(store).unwrap();
    let seller = ActorId::new("human:seller");
    let buyer = ActorId::new("human:buyer");
    for (id, name) in [(seller.clone(), "Seller"), (buyer.clone(), "Buyer")] {
        service
            .handle(
                ClientEnvelope::new(
                    context(&id.0),
                    ClientCommand::UpsertProfile {
                        actor: Actor::human(id.0.clone(), name),
                    },
                ),
                1,
            )
            .unwrap();
    }
    service
        .handle(
            ClientEnvelope::new(
                context(&seller.0),
                ClientCommand::CreateConversation {
                    conversation: Conversation::direct(
                        "chat:wallet-shop",
                        "Wallet shop",
                        vec![
                            Participant {
                                actor_id: seller.clone(),
                                role: ParticipantRole::Owner,
                                joined_at_ms: 1,
                                muted_until_ms: None,
                            },
                            Participant {
                                actor_id: buyer.clone(),
                                role: ParticipantRole::Member,
                                joined_at_ms: 1,
                                muted_until_ms: None,
                            },
                        ],
                        1,
                    ),
                },
            ),
            2,
        )
        .unwrap();
    let invoice = Invoice {
        id: "invoice:wallet:1".into(),
        conversation_id: ConversationId::new("chat:wallet-shop"),
        seller_id: seller.clone(),
        title: "Pro".into(),
        description: "Digital entitlement".into(),
        kind: InvoiceKind::DigitalGoods,
        currency: "USD".into(),
        prices: vec![PriceLine {
            label: "Pro".into(),
            amount: Money::new("USD", 250),
        }],
        payload: "product=pro".into(),
        provider_id: "fabushi-wallet".into(),
        start_parameter: None,
        request_name: false,
        request_email: false,
        request_phone: false,
        request_shipping_address: false,
        flexible_shipping: false,
        created_at_ms: 3,
        expires_at_ms: Some(10_000),
    };
    service
        .handle(
            ClientEnvelope::new(context(&seller.0), ClientCommand::CreateInvoice { invoice }),
            3,
        )
        .unwrap();
    service
        .credit_wallet_from_settlement(
            "settlement:buyer:1".into(),
            buyer.clone(),
            Money::new("USD", 1_000),
            Some("external:test".into()),
            4,
        )
        .unwrap();

    let checkout = service
        .handle(
            ClientEnvelope::new(
                context(&buyer.0),
                ClientCommand::CheckoutInvoice {
                    invoice_id: "invoice:wallet:1".into(),
                    order_id: "order:wallet:1".into(),
                    customer: None,
                },
            ),
            5,
        )
        .unwrap();
    let paid = checkout
        .iter()
        .find_map(|envelope| match &envelope.event {
            ServerEvent::OrderChanged { order } => Some(order),
            _ => None,
        })
        .expect("paid order event");
    assert_eq!(paid.status, PaymentStatus::Paid);
    assert_eq!(paid.amount, Money::new("USD", 250));

    let buyer_status = service
        .handle(
            ClientEnvelope::new(context(&buyer.0), ClientCommand::WalletStatus),
            6,
        )
        .unwrap();
    let buyer_account = match &buyer_status[0].event {
        ServerEvent::WalletStatus {
            account: Some(account),
            ..
        } => account,
        other => panic!("unexpected buyer wallet event: {other:?}"),
    };
    assert_eq!(buyer_account.balance("USD"), 750);

    let seller_status = service
        .handle(
            ClientEnvelope::new(context(&seller.0), ClientCommand::WalletStatus),
            6,
        )
        .unwrap();
    let seller_account = match &seller_status[0].event {
        ServerEvent::WalletStatus {
            account: Some(account),
            ..
        } => account,
        other => panic!("unexpected seller wallet event: {other:?}"),
    };
    assert_eq!(seller_account.balance("USD"), 250);

    let refund = service
        .handle(
            ClientEnvelope::new(
                context(&seller.0),
                ClientCommand::RefundOrder {
                    order_id: "order:wallet:1".into(),
                    request_id: "refund:wallet:1".into(),
                },
            ),
            7,
        )
        .unwrap();
    let refunded = refund
        .iter()
        .find_map(|envelope| match &envelope.event {
            ServerEvent::OrderChanged { order } => Some(order),
            _ => None,
        })
        .expect("refunded order event");
    assert_eq!(refunded.status, PaymentStatus::Refunded);

    service
        .handle(
            ClientEnvelope::new(
                context(&seller.0),
                ClientCommand::RefundOrder {
                    order_id: "order:wallet:1".into(),
                    request_id: "refund:wallet:retry".into(),
                },
            ),
            8,
        )
        .unwrap();
    let buyer_account_id = WalletAccountId("wallet:human:buyer".into());
    let seller_account_id = WalletAccountId("wallet:human:seller".into());
    assert_eq!(
        service.engine().state().wallet.accounts[&buyer_account_id].balance("USD"),
        1_000
    );
    assert_eq!(
        service.engine().state().wallet.accounts[&seller_account_id].balance("USD"),
        0
    );
}

#[test]
fn sync_is_isolated_to_authenticated_actor_conversations() {
    let store = MemoryStateStore::default();
    let mut service = MessagingService::load(store).unwrap();
    for (id, name) in [("human:a", "A"), ("human:b", "B")] {
        service
            .handle(
                ClientEnvelope::new(
                    context(id),
                    ClientCommand::UpsertProfile {
                        actor: Actor::human(id, name),
                    },
                ),
                1,
            )
            .unwrap();
        service
            .handle(
                ClientEnvelope::new(
                    context(id),
                    ClientCommand::CreateConversation {
                        conversation: Conversation::direct(
                            format!("saved:{id}"),
                            format!("Saved {name}"),
                            vec![Participant {
                                actor_id: ActorId::new(id),
                                role: ParticipantRole::Owner,
                                joined_at_ms: 1,
                                muted_until_ms: None,
                            }],
                            1,
                        ),
                    },
                ),
                2,
            )
            .unwrap();
        service
            .handle(
                ClientEnvelope::new(
                    context(id),
                    ClientCommand::SendMessage {
                        conversation_id: ConversationId::new(format!("saved:{id}")),
                        client_message_id: ClientMessageId(format!("client:{id}")),
                        content: MessageContent::Text {
                            text: FormattedText::plain(format!("private-{id}")),
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
    }

    let sync = service
        .handle(
            ClientEnvelope::new(
                context("human:a"),
                ClientCommand::Sync {
                    cursor: None,
                    limit: 100,
                },
            ),
            4,
        )
        .unwrap();
    match &sync[0].event {
        ServerEvent::SyncBatch {
            actors,
            conversations,
            messages,
            ..
        } => {
            assert!(actors
                .iter()
                .all(|actor| actor.id != ActorId::new("human:b")));
            assert_eq!(conversations.len(), 1);
            assert_eq!(conversations[0].id, ConversationId::new("saved:human:a"));
            assert_eq!(messages.len(), 1);
            assert!(matches!(
                &messages[0].content,
                MessageContent::Text { text } if text.text == "private-human:a"
            ));
        }
        other => panic!("unexpected sync event: {other:?}"),
    }
}

#[test]
fn authenticated_actor_cannot_spoof_profile_or_invoice_seller() {
    let store = MemoryStateStore::default();
    let mut service = MessagingService::load(store).unwrap();
    let error = service
        .handle(
            ClientEnvelope::new(
                context("human:a"),
                ClientCommand::UpsertProfile {
                    actor: Actor::human("human:b", "Spoofed"),
                },
            ),
            1,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        MessagingServiceError::UnauthorizedCommand(_)
    ));
}

#[test]
fn human_group_message_requests_bot_execution_inside_messaging_service() {
    let store = MemoryStateStore::default();
    let mut service = MessagingService::load(store).unwrap();
    service
        .handle(
            ClientEnvelope::new(
                context("human:caller"),
                ClientCommand::UpsertProfile {
                    actor: Actor::human("human:caller", "Caller"),
                },
            ),
            1,
        )
        .unwrap();
    service
        .handle(
            ClientEnvelope::new(
                context("bot:helper"),
                ClientCommand::UpsertProfile {
                    actor: Actor::bot("bot:helper", "Helper"),
                },
            ),
            1,
        )
        .unwrap();
    let mut conversation = Conversation::direct(
        "group:bot-runtime",
        "Bot runtime",
        vec![
            Participant {
                actor_id: ActorId::new("human:caller"),
                role: ParticipantRole::Owner,
                joined_at_ms: 1,
                muted_until_ms: None,
            },
            Participant {
                actor_id: ActorId::new("bot:helper"),
                role: ParticipantRole::Member,
                joined_at_ms: 1,
                muted_until_ms: None,
            },
        ],
        1,
    );
    conversation.kind = ConversationKind::Group;
    conversation.owner_id = Some(ActorId::new("human:caller"));
    service
        .handle(
            ClientEnvelope::new(
                context("human:caller"),
                ClientCommand::CreateConversation { conversation },
            ),
            2,
        )
        .unwrap();
    let responses = service
        .handle(
            ClientEnvelope::new(
                context("human:caller"),
                ClientCommand::SendMessage {
                    conversation_id: ConversationId::new("group:bot-runtime"),
                    client_message_id: ClientMessageId("client:bot-runtime".into()),
                    content: MessageContent::Text {
                        text: FormattedText::plain("/help 请处理"),
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
    let invocation = responses
        .iter()
        .find_map(|response| match &response.event {
            ServerEvent::BotInvocationRequested { invocation } => Some(invocation),
            _ => None,
        })
        .expect("bot invocation request");
    assert_eq!(invocation.bot_id, ActorId::new("bot:helper"));
    assert_eq!(invocation.sender_id, ActorId::new("human:caller"));
    assert_eq!(
        invocation.conversation_id,
        ConversationId::new("group:bot-runtime")
    );
    assert_eq!(invocation.command.as_deref(), Some("help"));
    assert_eq!(
        invocation.metadata.get("source").map(String::as_str),
        Some("messaging-service")
    );
}
