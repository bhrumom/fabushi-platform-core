use fabushi_messaging_core::*;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_root(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "fabushi-m4-{label}-{}-{suffix}",
        std::process::id()
    ))
}

fn media(id: &str, thumbnail_id: Option<&str>) -> MediaRef {
    MediaRef {
        id: id.into(),
        file_name: Some(format!("{id}.bin")),
        mime_type: Some("application/octet-stream".into()),
        size_bytes: Some(6),
        width: None,
        height: None,
        duration_ms: None,
        thumbnail_id: thumbnail_id.map(str::to_string),
        local_path: None,
        remote_url: None,
        content_hash: None,
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

#[test]
fn blob_store_resumes_ranges_and_verifies_sha256_before_publish() {
    let root = temporary_root("blob");
    let store = FileBlobStore::new(&root);
    let id = BlobId::new("media-contract").unwrap();
    let metadata = BlobMetadata {
        id: id.clone(),
        file_name: "hello.txt".into(),
        mime_type: "text/plain".into(),
        size_bytes: 11,
        content_hash: Some(
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9".into(),
        ),
        created_at_ms: 1,
    };

    assert_eq!(store.begin_upload(&metadata).unwrap().uploaded_bytes, 0);
    store.append_chunk(&id, 0, b"hello ").unwrap();
    assert_eq!(store.upload_status(&id).unwrap().uploaded_bytes, 6);
    store.append_chunk(&id, 6, b"world").unwrap();
    let finished = store.finish_upload(&id).unwrap();
    assert_eq!(finished, metadata);
    assert_eq!(store.read_range(&id, 6, 5).unwrap(), b"world");

    let bad_id = BlobId::new("media-contract-bad").unwrap();
    let bad = BlobMetadata {
        id: bad_id.clone(),
        file_name: "bad.txt".into(),
        mime_type: "text/plain".into(),
        size_bytes: 5,
        content_hash: Some("00".repeat(32)),
        created_at_ms: 2,
    };
    store.begin_upload(&bad).unwrap();
    store.append_chunk(&bad_id, 0, b"hello").unwrap();
    assert!(matches!(
        store.finish_upload(&bad_id),
        Err(BlobStoreError::IntegrityMismatch { .. })
    ));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn media_cache_is_bounded_lru_and_preserves_thumbnail_metadata() {
    let mut cache = MediaCache::new(10).unwrap();
    let first = media("first", Some("thumb:first"));
    cache
        .insert(first.clone(), "/cache/first", 6, 1, false)
        .unwrap();
    assert_eq!(
        cache.entries()["first"].media.thumbnail_id.as_deref(),
        Some("thumb:first")
    );

    let second = media("second", Some("thumb:second"));
    let evicted = cache
        .insert(second.clone(), "/cache/second", 6, 2, false)
        .unwrap();
    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].media.id, "first");
    assert!(cache.entries().contains_key("second"));
    assert!(cache.total_bytes() <= cache.max_bytes());

    cache.set_pinned("second", true).unwrap();
    assert_eq!(cache.get("second", 3).unwrap().last_accessed_ms, 3);
}

#[test]
fn media_and_poll_sends_follow_conversation_message_permissions() {
    let mut engine = MessagingEngine::new();
    for actor_id in ["human:owner", "human:member", "human:outsider"] {
        engine
            .execute(Command::UpsertActor {
                actor: Actor::human(actor_id, actor_id),
            })
            .unwrap();
    }

    let conversation_id = ConversationId::new("group:m4-permissions");
    let mut conversation = Conversation::direct(
        conversation_id.0.clone(),
        "M4 permissions",
        vec![
            participant("human:owner", ParticipantRole::Owner),
            participant("human:member", ParticipantRole::Member),
        ],
        1,
    );
    conversation.kind = ConversationKind::Group;
    conversation.owner_id = Some(ActorId::new("human:owner"));
    conversation.permissions.can_send_messages = true;
    conversation.permissions.can_send_media = false;
    conversation.permissions.can_send_polls = false;
    engine
        .execute(Command::UpsertConversation { conversation })
        .unwrap();

    let photo = MessageContent::Photo {
        media: media("photo:1", Some("thumb:photo:1")),
        caption: FormattedText::plain("photo"),
        spoiler: false,
    };
    assert!(matches!(
        engine.execute(Command::QueueMessage {
            conversation_id: conversation_id.clone(),
            local_message_id: MessageId::new("local:photo"),
            client_message_id: ClientMessageId("client:photo".into()),
            sender_id: ActorId::new("human:member"),
            content: photo,
            reply_to_message_id: None,
            thread_root_message_id: None,
            created_at_ms: 2,
            scheduled_at_ms: None,
            silent: false,
            protected_content: false,
        }),
        Err(EngineError::MediaSendPermissionDenied(id)) if id == conversation_id
    ));

    let poll = MessageContent::Poll {
        question: FormattedText::plain("choose"),
        options: vec![PollOption {
            id: "a".into(),
            text: "A".into(),
            voter_count: 0,
            chosen: false,
            correct: None,
        }],
        anonymous: true,
        multiple_answers: false,
        quiz: false,
    };
    assert!(matches!(
        engine.execute(Command::QueueMessage {
            conversation_id: conversation_id.clone(),
            local_message_id: MessageId::new("local:poll"),
            client_message_id: ClientMessageId("client:poll".into()),
            sender_id: ActorId::new("human:member"),
            content: poll,
            reply_to_message_id: None,
            thread_root_message_id: None,
            created_at_ms: 3,
            scheduled_at_ms: None,
            silent: false,
            protected_content: false,
        }),
        Err(EngineError::PollSendPermissionDenied(id)) if id == conversation_id
    ));

    assert!(matches!(
        engine.execute(Command::QueueMessage {
            conversation_id: conversation_id.clone(),
            local_message_id: MessageId::new("local:outsider"),
            client_message_id: ClientMessageId("client:outsider".into()),
            sender_id: ActorId::new("human:outsider"),
            content: MessageContent::Text {
                text: FormattedText::plain("not a member"),
            },
            reply_to_message_id: None,
            thread_root_message_id: None,
            created_at_ms: 4,
            scheduled_at_ms: None,
            silent: false,
            protected_content: false,
        }),
        Err(EngineError::SenderNotParticipant { conversation_id: id, .. }) if id == conversation_id
    ));
}
