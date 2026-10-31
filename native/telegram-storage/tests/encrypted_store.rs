use fabushi_telegram_core::{
    Chat, ChatId, ChatKind, Command, DeliveryState, Event, FormattedText, Message, MessageContent,
    MessageId, TelegramEngine, TelegramState, UserId,
};
use fabushi_telegram_storage::{EncryptedSqliteStore, StorageError, StorageKey};
use rusqlite::Connection;
use tempfile::tempdir;

fn key(byte: u8) -> StorageKey {
    StorageKey::from_slice(&[byte; 32]).unwrap()
}

fn text_message(id: i64, chat_id: i64, text: &str, date_unix_ms: i64) -> Message {
    Message {
        id: MessageId(id),
        chat_id: ChatId(chat_id),
        sender_user_id: UserId(99),
        date_unix_ms,
        edit_date_unix_ms: None,
        content: MessageContent::Text(FormattedText::plain(text)),
        reply_to_message_id: None,
        message_thread_id: None,
        delivery_state: DeliveryState::Sent,
        reactions: Vec::new(),
        is_outgoing: false,
        is_pinned: false,
        is_deleted: false,
    }
}

fn state_with_private_chat() -> TelegramState {
    let mut engine = TelegramEngine::new();
    engine
        .execute(Command::UpsertChat {
            chat: Chat::new(ChatId(42), ChatKind::Private, "只应出现在密文中"),
        })
        .unwrap();
    engine.into_state()
}

#[test]
fn snapshot_round_trips_and_uses_optimistic_revision() {
    let mut store = EncryptedSqliteStore::open_in_memory(key(1)).unwrap();
    let state = state_with_private_chat();
    let saved = store.save_snapshot(&state, 0, 1000).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(store.load_snapshot().unwrap().unwrap(), saved);

    let error = store.save_snapshot(&state, 0, 1001).unwrap_err();
    assert!(matches!(
        error,
        StorageError::RevisionConflict {
            expected: 0,
            current: 1
        }
    ));
}

#[test]
fn database_file_does_not_contain_plaintext_and_wrong_key_cannot_open_payload() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("telegram.sqlite3");
    {
        let mut store = EncryptedSqliteStore::open(&path, key(2)).unwrap();
        store
            .save_snapshot(&state_with_private_chat(), 0, 1000)
            .unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("只应出现在密文中"));

    let wrong_key_store = EncryptedSqliteStore::open(&path, key(3)).unwrap();
    assert!(matches!(
        wrong_key_store.load_snapshot().unwrap_err(),
        StorageError::DecryptionFailed
    ));
}

#[test]
fn encrypted_event_log_preserves_order_and_supports_pruning() {
    let mut store = EncryptedSqliteStore::open_in_memory(key(4)).unwrap();
    let first = Event::ChatUpserted {
        chat: Chat::new(ChatId(1), ChatKind::Private, "一"),
    };
    let second = Event::ChatUpserted {
        chat: Chat::new(ChatId(2), ChatKind::Channel, "二"),
    };
    assert_eq!(store.append_event(&first, 10).unwrap().sequence, 1);
    assert_eq!(store.append_event(&second, 20).unwrap().sequence, 2);

    let events = store.load_events_after(0).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event, first);
    assert_eq!(events[1].event, second);
    assert_eq!(store.load_events_after(1).unwrap().len(), 1);
    assert_eq!(store.prune_events_through(1).unwrap(), 1);
    assert_eq!(store.load_events_after(0).unwrap().len(), 1);
}

#[test]
fn transition_commits_events_and_snapshot_under_one_revision() {
    let mut store = EncryptedSqliteStore::open_in_memory(key(6)).unwrap();
    let state = state_with_private_chat();
    let event = Event::ChatUpserted {
        chat: Chat::new(ChatId(42), ChatKind::Private, "原子提交"),
    };
    let committed = store
        .commit_transition(&state, std::slice::from_ref(&event), 0, 3000)
        .unwrap();
    assert_eq!(committed.snapshot.revision, 1);
    assert_eq!(committed.events.len(), 1);
    assert_eq!(committed.events[0].event, event);
    assert_eq!(store.load_snapshot().unwrap().unwrap().state, state);
    assert_eq!(store.load_events_after(0).unwrap().len(), 1);

    let conflict = store.commit_transition(&TelegramState::default(), &[], 0, 3001);
    assert!(matches!(
        conflict,
        Err(StorageError::RevisionConflict {
            expected: 0,
            current: 1
        })
    ));
    assert_eq!(store.load_events_after(0).unwrap().len(), 1);
}

#[test]
fn encrypted_message_index_supports_cjk_multi_token_search_chat_scope_and_updates() {
    let mut store = EncryptedSqliteStore::open_in_memory(key(8)).unwrap();
    let first = text_message(1, 42, "佛法修行 Telegram Rust", 1000);
    let second = text_message(2, 7, "佛法在另一个会话", 2000);
    let third = text_message(3, 42, "只有 Rust", 3000);
    for message in [&first, &second, &third] {
        store.upsert_message_index(message).unwrap();
    }

    let all_chats = store.search_messages("佛法", None, 20).unwrap();
    assert_eq!(
        all_chats
            .iter()
            .map(|message| message.id.0)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    let chat_only = store.search_messages("佛法", Some(ChatId(42)), 20).unwrap();
    assert_eq!(chat_only, vec![first.clone()]);
    assert_eq!(
        store.search_messages("telegram rust", None, 20).unwrap(),
        vec![first.clone()]
    );
    assert_eq!(
        store
            .load_indexed_message(ChatId(42), MessageId(1))
            .unwrap(),
        Some(first.clone())
    );

    let updated = text_message(1, 42, "已经更新的索引内容", 4000);
    store.upsert_message_index(&updated).unwrap();
    assert!(store
        .search_messages("telegram", None, 20)
        .unwrap()
        .is_empty());
    assert_eq!(
        store.search_messages("更新", None, 20).unwrap(),
        vec![updated.clone()]
    );

    let mut deleted = updated;
    deleted.is_deleted = true;
    store.upsert_message_index(&deleted).unwrap();
    assert!(store.search_messages("更新", None, 20).unwrap().is_empty());
    assert_eq!(
        store
            .load_indexed_message(ChatId(42), MessageId(1))
            .unwrap(),
        None
    );
}

#[test]
fn message_payload_and_search_terms_are_not_written_as_plaintext() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("message-index.sqlite3");
    {
        let mut store = EncryptedSqliteStore::open(&path, key(9)).unwrap();
        store
            .upsert_message_index(&text_message(
                11,
                42,
                "绝不能出现在数据库里的 searchable-secret",
                1000,
            ))
            .unwrap();
    }

    let bytes = std::fs::read(path).unwrap();
    let database_text = String::from_utf8_lossy(&bytes);
    assert!(!database_text.contains("绝不能出现在数据库里"));
    assert!(!database_text.contains("searchable-secret"));
}

#[test]
fn schema_v1_database_migrates_to_encrypted_message_index() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("schema-v1.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE state_snapshot (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               revision INTEGER NOT NULL CHECK (revision > 0),
               payload BLOB NOT NULL,
               updated_at_unix_ms INTEGER NOT NULL
             );
             CREATE TABLE event_log (
               sequence INTEGER PRIMARY KEY AUTOINCREMENT,
               payload BLOB NOT NULL,
               created_at_unix_ms INTEGER NOT NULL
             );
             PRAGMA user_version = 1;",
        )
        .unwrap();
    drop(connection);

    let mut store = EncryptedSqliteStore::open(&path, key(10)).unwrap();
    let message = text_message(12, 42, "迁移后的消息索引", 1000);
    store.upsert_message_index(&message).unwrap();
    assert_eq!(
        store.search_messages("消息索引", None, 10).unwrap(),
        vec![message]
    );
}

#[test]
fn newer_database_schema_is_rejected() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("future.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);

    let result = EncryptedSqliteStore::open(&path, key(5));
    assert!(matches!(
        result,
        Err(StorageError::UnsupportedSchemaVersion(99))
    ));
}


#[test]
fn secure_runtime_value_round_trips_encrypted_and_can_be_cleared() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("secure-runtime.sqlite3");
    let secret = br#"{"phoneNumber":"+15551234567","phoneCodeHash":"never-plaintext"}"#;
    {
        let mut store = EncryptedSqliteStore::open(&path, key(11)).unwrap();
        store
            .save_secure_value("authorization-runtime-v1", secret)
            .unwrap();
        assert_eq!(
            store
                .load_secure_value("authorization-runtime-v1")
                .unwrap()
                .as_deref(),
            Some(secret.as_slice())
        );
    }
    let bytes = std::fs::read(&path).unwrap();
    let database_text = String::from_utf8_lossy(&bytes);
    assert!(!database_text.contains("+15551234567"));
    assert!(!database_text.contains("never-plaintext"));

    let mut store = EncryptedSqliteStore::open(&path, key(11)).unwrap();
    assert!(store
        .delete_secure_value("authorization-runtime-v1")
        .unwrap());
    assert!(store
        .load_secure_value("authorization-runtime-v1")
        .unwrap()
        .is_none());
}

#[test]
fn schema_v2_database_migrates_to_secure_runtime_value_slot() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("schema-v2.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE state_snapshot (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               revision INTEGER NOT NULL CHECK (revision > 0),
               payload BLOB NOT NULL,
               updated_at_unix_ms INTEGER NOT NULL
             );
             CREATE TABLE event_log (
               sequence INTEGER PRIMARY KEY AUTOINCREMENT,
               payload BLOB NOT NULL,
               created_at_unix_ms INTEGER NOT NULL
             );
             CREATE TABLE message_index (
               chat_id INTEGER NOT NULL,
               message_id INTEGER NOT NULL,
               date_unix_ms INTEGER NOT NULL,
               payload BLOB NOT NULL,
               PRIMARY KEY (chat_id, message_id)
             );
             CREATE TABLE message_search_token (
               token BLOB NOT NULL,
               chat_id INTEGER NOT NULL,
               message_id INTEGER NOT NULL,
               PRIMARY KEY (token, chat_id, message_id)
             ) WITHOUT ROWID;
             PRAGMA user_version = 2;",
        )
        .unwrap();
    drop(connection);

    let mut store = EncryptedSqliteStore::open(&path, key(12)).unwrap();
    store
        .save_secure_value("authorization-runtime-v1", b"pending-auth")
        .unwrap();
    assert_eq!(
        store
            .load_secure_value("authorization-runtime-v1")
            .unwrap()
            .as_deref(),
        Some(b"pending-auth".as_slice())
    );
}


#[test]
fn schema_v3_private_metadata_database_migrates_to_unified_v4() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("schema-v3-private.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE private_metadata (
               key TEXT PRIMARY KEY,
               payload BLOB NOT NULL,
               updated_at_unix_ms INTEGER NOT NULL
             ) WITHOUT ROWID;
             PRAGMA user_version = 3;",
        )
        .unwrap();
    drop(connection);

    let mut store = EncryptedSqliteStore::open(&path, key(13)).unwrap();
    store
        .save_private_metadata("authorization.v1", b"identity-after-v3-migration", 7)
        .unwrap();
    store
        .save_secure_value("authorization-runtime-v1", b"lifecycle-after-v3-migration")
        .unwrap();
    assert_eq!(
        store.load_private_metadata("authorization.v1").unwrap().as_deref(),
        Some(b"identity-after-v3-migration".as_slice())
    );
    assert_eq!(
        store
            .load_secure_value("authorization-runtime-v1")
            .unwrap()
            .as_deref(),
        Some(b"lifecycle-after-v3-migration".as_slice())
    );
}

#[test]
fn schema_v3_secure_value_database_migrates_to_unified_v4() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("schema-v3-secure.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE secure_value (
               key TEXT PRIMARY KEY,
               payload BLOB NOT NULL
             ) WITHOUT ROWID;
             PRAGMA user_version = 3;",
        )
        .unwrap();
    drop(connection);

    let mut store = EncryptedSqliteStore::open(&path, key(15)).unwrap();
    store
        .save_secure_value("authorization-runtime-v1", b"secure-after-v3-migration")
        .unwrap();
    store
        .save_private_metadata("transport.v1", b"transport-after-v3-migration", 9)
        .unwrap();
    assert_eq!(
        store
            .load_secure_value("authorization-runtime-v1")
            .unwrap()
            .as_deref(),
        Some(b"secure-after-v3-migration".as_slice())
    );
    assert_eq!(
        store.load_private_metadata("transport.v1").unwrap().as_deref(),
        Some(b"transport-after-v3-migration".as_slice())
    );
}

#[test]
fn unified_v4_storage_keeps_private_metadata_and_secure_value_encrypted() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("schema-v4-unified.sqlite3");
    {
        let mut store = EncryptedSqliteStore::open(&path, key(14)).unwrap();
        store
            .save_private_metadata("transport.v1", b"private-transport-secret", 8)
            .unwrap();
        store
            .save_secure_value("authorization-runtime-v1", b"secure-auth-secret")
            .unwrap();
        assert_eq!(
            store.load_private_metadata("transport.v1").unwrap().as_deref(),
            Some(b"private-transport-secret".as_slice())
        );
        assert_eq!(
            store
                .load_secure_value("authorization-runtime-v1")
                .unwrap()
                .as_deref(),
            Some(b"secure-auth-secret".as_slice())
        );
    }
    let bytes = std::fs::read(path).unwrap();
    let database_text = String::from_utf8_lossy(&bytes);
    assert!(!database_text.contains("private-transport-secret"));
    assert!(!database_text.contains("secure-auth-secret"));
}
