use crate::actor::{ActorId, ActorKind};
use crate::blob_store::{BlobStoreError, FileBlobStore};
use crate::bot::BotInvocation;
use crate::conversation::{Conversation, ConversationId, ConversationKind};
use crate::engine::{Command, EngineError, Event, MessagingEngine};
use crate::message::{ClientMessageId, DeliveryState, Message, MessageContent, MessageId};
use crate::payment::Money;
use crate::protocol::{
    ClientCommand, ClientEnvelope, ServerEnvelope, ServerEvent, FABUSHI_MESSAGING_PROTOCOL_VERSION,
};
use crate::search::{SearchIndex, SearchQuery};
use crate::settlement::{SettlementError, SettlementVerifier, SignedSettlement};
use crate::store::{JournalEntry, MessagingSnapshot, MessagingStateStore, StoreError};
use crate::wallet::{LedgerEntry, WalletAccountId};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use thiserror::Error;

const TYPING_TTL_MS: i64 = 5_000;

#[derive(Debug, Error)]
pub enum MessagingServiceError {
    #[error("unsupported messaging protocol version {actual}; expected {expected}")]
    ProtocolVersion { expected: u16, actual: u16 },
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Blob(#[from] BlobStoreError),
    #[error("blob storage is unavailable for this messaging service")]
    BlobStoreUnavailable,
    #[error("blob chunk is not valid base64: {0}")]
    InvalidBlobBase64(String),
    #[error("messaging service invariant failed: {0}")]
    Invariant(String),
    #[error("messaging command is not authorized for the authenticated actor: {0}")]
    UnauthorizedCommand(String),
    #[error("client message id {0} was replayed with conflicting message content")]
    IdempotencyConflict(String),
    #[error(transparent)]
    Settlement(#[from] SettlementError),
}

fn sanitize_invocation_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':') {
                character
            } else {
                '_'
            }
        })
        .take(160)
        .collect()
}

fn stable_message_id(actor_id: &ActorId, client_message_id: &ClientMessageId) -> MessageId {
    let mut hasher = Sha256::new();
    hasher.update(actor_id.0.as_bytes());
    hasher.update([0]);
    hasher.update(client_message_id.0.as_bytes());
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(encoded, "{byte:02x}");
    }
    MessageId::new(format!("msg:{encoded}"))
}

pub struct MessagingService<S: MessagingStateStore> {
    engine: MessagingEngine,
    store: S,
    blob_store: Option<FileBlobStore>,
    cursor: u64,
}

impl<S: MessagingStateStore> MessagingService<S> {
    pub fn load(store: S) -> Result<Self, MessagingServiceError> {
        let snapshot = store.load()?;
        let (engine, cursor) = match snapshot {
            Some(snapshot) => (MessagingEngine::from_state(snapshot.state), snapshot.cursor),
            None => (MessagingEngine::new(), 0),
        };
        Ok(Self {
            engine,
            store,
            blob_store: None,
            cursor,
        })
    }

    pub fn load_with_blob_store(
        store: S,
        blob_store: FileBlobStore,
    ) -> Result<Self, MessagingServiceError> {
        let mut service = Self::load(store)?;
        service.blob_store = Some(blob_store);
        Ok(service)
    }

    pub fn engine(&self) -> &MessagingEngine {
        &self.engine
    }

    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    pub fn into_store(self) -> S {
        self.store
    }

    pub fn handle(
        &mut self,
        envelope: ClientEnvelope,
        server_time_ms: i64,
    ) -> Result<Vec<ServerEnvelope>, MessagingServiceError> {
        if envelope.protocol_version != FABUSHI_MESSAGING_PROTOCOL_VERSION {
            return Err(MessagingServiceError::ProtocolVersion {
                expected: FABUSHI_MESSAGING_PROTOCOL_VERSION,
                actual: envelope.protocol_version,
            });
        }

        let actor_id = envelope.context.actor_id;
        let command = envelope.command;
        self.validate_command_authorization(&actor_id, &command)?;
        match command {
            ClientCommand::BeginBlobUpload { metadata } => {
                let status = self.blob_store()?.begin_upload(&metadata)?;
                self.single_service_event(
                    &actor_id,
                    ServerEvent::BlobUploadChanged { status },
                    server_time_ms,
                )
            }
            ClientCommand::AppendBlobChunk {
                blob_id,
                offset,
                data_base64,
            } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_base64.as_bytes())
                    .map_err(|error| MessagingServiceError::InvalidBlobBase64(error.to_string()))?;
                let status = self.blob_store()?.append_chunk(&blob_id, offset, &bytes)?;
                self.single_service_event(
                    &actor_id,
                    ServerEvent::BlobUploadChanged { status },
                    server_time_ms,
                )
            }
            ClientCommand::FinishBlobUpload { blob_id } => {
                let metadata = self.blob_store()?.finish_upload(&blob_id)?;
                self.single_service_event(
                    &actor_id,
                    ServerEvent::BlobReady { metadata },
                    server_time_ms,
                )
            }
            ClientCommand::DeleteBlob { blob_id } => {
                self.blob_store()?.delete(&blob_id)?;
                self.single_service_event(
                    &actor_id,
                    ServerEvent::BlobDeleted { blob_id },
                    server_time_ms,
                )
            }
            ClientCommand::WalletStatus => {
                Ok(vec![self.wallet_status_envelope(&actor_id, server_time_ms)])
            }
            ClientCommand::Sync { cursor, limit } => {
                self.mark_direct_messages_delivered(&actor_id, server_time_ms)?;
                self.sync_response(&actor_id, cursor.as_deref(), limit, server_time_ms)
            }
            ClientCommand::Search { query } => {
                Ok(vec![self.search_envelope(&actor_id, query, server_time_ms)])
            }
            ClientCommand::StartTyping {
                conversation_id,
                action,
            } => self.typing_event(
                &actor_id,
                conversation_id,
                Some(action),
                Some(server_time_ms.saturating_add(TYPING_TTL_MS)),
                server_time_ms,
            ),
            ClientCommand::StopTyping { conversation_id } => self.typing_event(
                &actor_id,
                conversation_id,
                None,
                Some(server_time_ms.saturating_add(TYPING_TTL_MS)),
                server_time_ms,
            ),
            command => {
                if let Some(replay) =
                    self.idempotent_send_replay(&actor_id, &command, server_time_ms)?
                {
                    return Ok(replay);
                }

                let commands = self.project_command(&actor_id, command, server_time_ms);
                let mut events = Vec::new();
                for command in commands {
                    events.extend(self.engine.execute(command)?);
                }
                if events.is_empty() {
                    return Ok(Vec::new());
                }

                let bot_invocations = events
                    .iter()
                    .filter_map(|event| match event {
                        Event::MessageQueued { message } => Some(message),
                        _ => None,
                    })
                    .flat_map(|message| self.bot_invocations_for_message(message))
                    .collect::<Vec<_>>();
                self.cursor = self.cursor.saturating_add(events.len() as u64);
                let mut responses = events
                    .into_iter()
                    .filter_map(|event| self.project_event(event, server_time_ms))
                    .collect::<Vec<_>>();
                responses.extend(
                    bot_invocations
                        .into_iter()
                        .map(|invocation| ServerEnvelope {
                            protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
                            cursor: Some(self.cursor.to_string()),
                            server_time_ms,
                            event: ServerEvent::BotInvocationRequested { invocation },
                        }),
                );
                let journal = self.journal_entries(&actor_id, &responses);
                self.persist_with_events(server_time_ms, &journal)?;
                Ok(responses)
            }
        }
    }

    fn search_envelope(
        &self,
        actor_id: &ActorId,
        query: SearchQuery,
        server_time_ms: i64,
    ) -> ServerEnvelope {
        let state = self.engine.state();
        let visible_conversations = state
            .conversations
            .values()
            .filter(|conversation| {
                conversation.owner_id.as_ref() == Some(actor_id)
                    || conversation
                        .participants
                        .iter()
                        .any(|participant| &participant.actor_id == actor_id)
                    || state
                        .communities
                        .get(&conversation.id)
                        .and_then(|community| community.public_username.as_ref())
                        .is_some()
            })
            .map(|conversation| conversation.id.clone())
            .collect::<BTreeSet<_>>();
        let mut index = SearchIndex::default();
        for actor in state.actors.values().cloned() {
            index.index_actor(actor);
        }
        for conversation in state
            .conversations
            .values()
            .filter(|conversation| visible_conversations.contains(&conversation.id))
            .cloned()
        {
            index.index_conversation(conversation);
        }
        for message in visible_conversations
            .iter()
            .filter_map(|conversation_id| state.messages.get(conversation_id))
            .flat_map(|messages| messages.values())
            .filter(|message| !message.deleted)
            .cloned()
        {
            index.index_message(message);
        }
        let results = index.search(&query);
        ServerEnvelope {
            protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
            cursor: Some(self.cursor.to_string()),
            server_time_ms,
            event: ServerEvent::SearchResults { query, results },
        }
    }

    fn typing_event(
        &mut self,
        actor_id: &ActorId,
        conversation_id: ConversationId,
        action: Option<String>,
        expires_at_ms: Option<i64>,
        server_time_ms: i64,
    ) -> Result<Vec<ServerEnvelope>, MessagingServiceError> {
        let conversation = self
            .engine
            .state()
            .conversations
            .get(&conversation_id)
            .ok_or_else(|| {
                MessagingServiceError::UnauthorizedCommand(
                    "typing conversation does not exist".into(),
                )
            })?;
        if !conversation
            .participants
            .iter()
            .any(|participant| &participant.actor_id == actor_id)
        {
            return Err(MessagingServiceError::UnauthorizedCommand(
                "typing requires conversation membership".into(),
            ));
        }
        self.cursor = self.cursor.saturating_add(1);
        let response = ServerEnvelope {
            protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
            cursor: Some(self.cursor.to_string()),
            server_time_ms,
            event: ServerEvent::TypingChanged {
                conversation_id,
                actor_id: actor_id.clone(),
                action,
                expires_at_ms,
            },
        };
        let journal = self.journal_entries(actor_id, std::slice::from_ref(&response));
        self.persist_with_events(server_time_ms, &journal)?;
        Ok(vec![response])
    }

    fn idempotent_send_replay(
        &self,
        actor_id: &ActorId,
        command: &ClientCommand,
        server_time_ms: i64,
    ) -> Result<Option<Vec<ServerEnvelope>>, MessagingServiceError> {
        let ClientCommand::SendMessage {
            conversation_id,
            client_message_id,
            content,
            reply_to_message_id,
            thread_root_message_id,
            scheduled_at_ms,
            silent,
            protected_content,
        } = command
        else {
            return Ok(None);
        };
        let stable_id = stable_message_id(actor_id, client_message_id);
        let legacy_id = MessageId::new(format!("local:{}", client_message_id.0));
        let existing = self
            .engine
            .state()
            .messages
            .get(conversation_id)
            .and_then(|messages| {
                messages
                    .get(&stable_id)
                    .or_else(|| messages.get(&legacy_id))
            });
        let Some(existing) = existing else {
            return Ok(None);
        };
        // The stable/legacy lookup key already binds this replay to the authenticated
        // actor + client_message_id, without extending the canonical Message schema.
        if &existing.sender_id != actor_id
            || &existing.content != content
            || &existing.reply_to_message_id != reply_to_message_id
            || &existing.thread_root_message_id != thread_root_message_id
            || &existing.scheduled_at_ms != scheduled_at_ms
            || &existing.silent != silent
            || &existing.protected_content != protected_content
        {
            return Err(MessagingServiceError::IdempotencyConflict(
                client_message_id.0.clone(),
            ));
        }
        Ok(Some(vec![ServerEnvelope {
            protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
            cursor: Some(self.cursor.to_string()),
            server_time_ms,
            event: ServerEvent::MessageChanged {
                message: existing.clone(),
            },
        }]))
    }

    fn bot_invocations_for_message(&self, message: &Message) -> Vec<BotInvocation> {
        let sender = match self.engine.state().actors.get(&message.sender_id) {
            Some(sender) => sender,
            None => return Vec::new(),
        };
        if matches!(
            sender.kind,
            ActorKind::Bot | ActorKind::Assistant | ActorKind::Service
        ) {
            return Vec::new();
        }
        let MessageContent::Text { text } = &message.content else {
            return Vec::new();
        };
        let Some(conversation) = self
            .engine
            .state()
            .conversations
            .get(&message.conversation_id)
        else {
            return Vec::new();
        };
        let command = text
            .text
            .trim()
            .strip_prefix('/')
            .and_then(|value| value.split_whitespace().next())
            .map(|value| value.trim_start_matches('@').to_string())
            .filter(|value| !value.is_empty());
        conversation
            .participants
            .iter()
            .filter_map(|participant| {
                if participant.actor_id == message.sender_id {
                    return None;
                }
                let actor = self.engine.state().actors.get(&participant.actor_id)?;
                if !matches!(actor.kind, ActorKind::Bot | ActorKind::Assistant) {
                    return None;
                }
                Some(BotInvocation {
                    id: format!(
                        "invoke:auto:{}:{}",
                        sanitize_invocation_component(&message.id.0),
                        sanitize_invocation_component(&actor.id.0)
                    ),
                    bot_id: actor.id.clone(),
                    sender_id: message.sender_id.clone(),
                    conversation_id: message.conversation_id.clone(),
                    command: command.clone(),
                    text: text.clone(),
                    reply_to_message_id: message
                        .reply_to_message_id
                        .as_ref()
                        .map(|id| id.0.clone()),
                    metadata: std::collections::BTreeMap::from([
                        ("source".into(), "messaging-service".into()),
                        ("messageId".into(), message.id.0.clone()),
                    ]),
                    created_at_ms: message.created_at_ms,
                })
            })
            .collect()
    }

    fn validate_command_authorization(
        &self,
        actor_id: &ActorId,
        command: &ClientCommand,
    ) -> Result<(), MessagingServiceError> {
        let denied = |reason: &str| MessagingServiceError::UnauthorizedCommand(reason.into());
        match command {
            ClientCommand::UpsertProfile { actor } if &actor.id != actor_id => {
                return Err(denied(
                    "profile actor id does not match authenticated actor",
                ));
            }
            ClientCommand::CreateConversation { conversation } => {
                let caller_is_participant = conversation
                    .participants
                    .iter()
                    .any(|participant| &participant.actor_id == actor_id);
                if !caller_is_participant
                    || conversation
                        .owner_id
                        .as_ref()
                        .is_some_and(|owner| owner != actor_id)
                {
                    return Err(denied("conversation creator must be an owner/participant"));
                }
            }
            ClientCommand::UpdateConversation { conversation } => {
                let existing = self
                    .engine
                    .state()
                    .conversations
                    .get(&conversation.id)
                    .ok_or_else(|| denied("conversation update target does not exist"))?;
                let caller = existing
                    .participants
                    .iter()
                    .find(|participant| &participant.actor_id == actor_id)
                    .ok_or_else(|| denied("conversation update requires membership"))?;
                if !matches!(
                    caller.role,
                    crate::actor::ParticipantRole::Owner | crate::actor::ParticipantRole::Admin
                ) {
                    return Err(denied("conversation update requires owner/admin role"));
                }
            }
            ClientCommand::CreateInvoice { invoice } if &invoice.seller_id != actor_id => {
                return Err(denied(
                    "invoice seller id does not match authenticated actor",
                ));
            }
            ClientCommand::GrantMiniApp { grant } if &grant.actor_id != actor_id => {
                return Err(denied(
                    "Mini App grant actor does not match authenticated actor",
                ));
            }
            ClientCommand::OpenMiniApp { session } if &session.actor_id != actor_id => {
                return Err(denied(
                    "Mini App session actor does not match authenticated actor",
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn blob_store(&self) -> Result<&FileBlobStore, MessagingServiceError> {
        self.blob_store
            .as_ref()
            .ok_or(MessagingServiceError::BlobStoreUnavailable)
    }

    fn single_service_event(
        &mut self,
        actor_id: &ActorId,
        event: ServerEvent,
        server_time_ms: i64,
    ) -> Result<Vec<ServerEnvelope>, MessagingServiceError> {
        self.cursor = self.cursor.saturating_add(1);
        let response = ServerEnvelope {
            protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
            cursor: Some(self.cursor.to_string()),
            server_time_ms,
            event,
        };
        let journal = self.journal_entries(actor_id, std::slice::from_ref(&response));
        self.persist_with_events(server_time_ms, &journal)?;
        Ok(vec![response])
    }

    pub fn apply_signed_settlement(
        &mut self,
        verifier: &SettlementVerifier,
        signed: &SignedSettlement,
        server_time_ms: i64,
    ) -> Result<LedgerEntry, MessagingServiceError> {
        let event = verifier.verify(signed, server_time_ms)?;
        self.credit_wallet_from_settlement(
            event.idempotency_key(),
            event.actor_id,
            event.amount,
            Some(event.provider_reference),
            server_time_ms,
        )
    }

    pub fn credit_wallet_from_settlement(
        &mut self,
        request_id: String,
        owner_id: ActorId,
        amount: Money,
        reference: Option<String>,
        server_time_ms: i64,
    ) -> Result<LedgerEntry, MessagingServiceError> {
        let events = self.engine.execute(Command::CreditWalletSettlement {
            request_id,
            owner_id,
            amount,
            reference,
            settled_at_ms: server_time_ms,
        })?;
        let entry = events
            .iter()
            .find_map(|event| match event {
                Event::WalletChanged { entry, .. } => Some(entry.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                MessagingServiceError::Invariant(
                    "wallet settlement produced no ledger entry".into(),
                )
            })?;
        self.cursor = self.cursor.saturating_add(events.len() as u64);
        self.persist(server_time_ms)?;
        Ok(entry)
    }

    fn wallet_status_envelope(&self, actor_id: &ActorId, server_time_ms: i64) -> ServerEnvelope {
        let account_id = WalletAccountId(format!("wallet:{}", actor_id.0));
        let account = self
            .engine
            .state()
            .wallet
            .accounts
            .get(&account_id)
            .cloned();
        let mut recent_entries = self
            .engine
            .state()
            .wallet
            .entries
            .values()
            .filter(|entry| {
                entry.from_account_id.as_ref() == Some(&account_id)
                    || entry.to_account_id.as_ref() == Some(&account_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        recent_entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at_ms));
        recent_entries.truncate(50);
        ServerEnvelope {
            protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
            cursor: Some(self.cursor.to_string()),
            server_time_ms,
            event: ServerEvent::WalletStatus {
                account,
                recent_entries,
            },
        }
    }

    fn sync_response(
        &self,
        actor_id: &ActorId,
        cursor: Option<&str>,
        limit: u32,
        server_time_ms: i64,
    ) -> Result<Vec<ServerEnvelope>, MessagingServiceError> {
        let Some(requested_cursor) = cursor.and_then(|value| value.parse::<u64>().ok()) else {
            return Ok(vec![self.sync_envelope(actor_id, limit, server_time_ms)]);
        };
        if requested_cursor > self.cursor {
            return Ok(vec![self.sync_envelope(actor_id, limit, server_time_ms)]);
        }
        let Some(slice) = self.store.load_event_journal_after(
            requested_cursor,
            usize::try_from(limit.max(1)).unwrap_or(usize::MAX),
        )?
        else {
            return Ok(vec![self.sync_envelope(actor_id, limit, server_time_ms)]);
        };
        if requested_cursor < slice.floor_cursor
            || (slice.entries.is_empty() && requested_cursor < slice.current_cursor)
        {
            return Ok(vec![self.sync_envelope(actor_id, limit, server_time_ms)]);
        }
        let mut responses = slice
            .entries
            .into_iter()
            .filter(|entry| entry.audience.iter().any(|candidate| candidate == actor_id))
            .filter(|entry| match &entry.envelope.event {
                ServerEvent::TypingChanged {
                    expires_at_ms: Some(expires_at_ms),
                    ..
                } => *expires_at_ms > server_time_ms,
                _ => true,
            })
            .map(|entry| entry.envelope)
            .collect::<Vec<_>>();
        responses.push(self.sync_checkpoint_envelope(slice.checkpoint_cursor, server_time_ms));
        Ok(responses)
    }

    fn sync_checkpoint_envelope(&self, cursor: u64, server_time_ms: i64) -> ServerEnvelope {
        ServerEnvelope {
            protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
            cursor: Some(cursor.to_string()),
            server_time_ms,
            event: ServerEvent::SyncBatch {
                actors: Vec::new(),
                conversations: Vec::new(),
                messages: Vec::new(),
                folders: Vec::new(),
                invoices: Vec::new(),
                orders: Vec::new(),
                stories: Vec::new(),
                communities: Vec::new(),
                bots: Vec::new(),
                bot_executions: Vec::new(),
                mini_apps: Vec::new(),
                next_cursor: Some(cursor.to_string()),
            },
        }
    }

    fn project_conversation_for_actor(
        &self,
        actor_id: &ActorId,
        conversation: &Conversation,
        server_time_ms: i64,
    ) -> Conversation {
        let state = self.engine.state();
        let mut projected = conversation.clone();
        let last_read = state
            .read_cursors
            .get(&conversation.id)
            .and_then(|cursors| cursors.get(actor_id));
        projected.last_read_message_id = last_read.map(|message_id| message_id.0.clone());

        let mut ordered_messages = state
            .messages
            .get(&conversation.id)
            .into_iter()
            .flat_map(|messages| messages.values())
            .collect::<Vec<_>>();
        ordered_messages.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        let read_index = last_read.and_then(|message_id| {
            ordered_messages
                .iter()
                .position(|message| &message.id == message_id)
        });
        let unread = ordered_messages
            .iter()
            .enumerate()
            .filter(|(index, message)| {
                let after_read = match read_index {
                    Some(read_index) => *index > read_index,
                    None => true,
                };
                let scheduled_is_visible = match message.scheduled_at_ms {
                    Some(scheduled_at_ms) => scheduled_at_ms <= server_time_ms,
                    None => true,
                };
                after_read
                    && scheduled_is_visible
                    && !message.deleted
                    && &message.sender_id != actor_id
            })
            .count();
        projected.unread_count = u32::try_from(unread).unwrap_or(u32::MAX);
        projected
    }

    fn sync_envelope(&self, actor_id: &ActorId, limit: u32, server_time_ms: i64) -> ServerEnvelope {
        let state = self.engine.state();
        let max_items = usize::try_from(limit.max(1)).unwrap_or(usize::MAX);
        let visible_conversation_ids = state
            .conversations
            .values()
            .filter(|conversation| {
                conversation
                    .participants
                    .iter()
                    .any(|participant| &participant.actor_id == actor_id)
                    || conversation.owner_id.as_ref() == Some(actor_id)
            })
            .map(|conversation| conversation.id.clone())
            .collect::<BTreeSet<_>>();
        let visible_actor_ids = visible_conversation_ids
            .iter()
            .filter_map(|conversation_id| state.conversations.get(conversation_id))
            .flat_map(|conversation| {
                conversation
                    .participants
                    .iter()
                    .map(|participant| participant.actor_id.clone())
            })
            .chain(std::iter::once(actor_id.clone()))
            .collect::<BTreeSet<_>>();
        ServerEnvelope {
            protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
            cursor: Some(self.cursor.to_string()),
            server_time_ms,
            event: ServerEvent::SyncBatch {
                // Actor/conversation metadata is the lightweight navigation index and must
                // be complete in a snapshot. Applying the message/event limit here used to
                // return only the first N contacts/conversations and then advance next_cursor
                // to the current journal position, so the omitted rows could never arrive via
                // later delta sync. Keep heavy collections bounded below, not the left-rail index.
                actors: visible_actor_ids
                    .iter()
                    .filter_map(|id| state.actors.get(id))
                    .cloned()
                    .collect(),
                conversations: visible_conversation_ids
                    .iter()
                    .filter_map(|id| state.conversations.get(id))
                    .map(|conversation| {
                        self.project_conversation_for_actor(actor_id, conversation, server_time_ms)
                    })
                    .collect(),
                messages: visible_conversation_ids
                    .iter()
                    .filter_map(|id| state.messages.get(id))
                    .flat_map(|messages| messages.values())
                    .take(max_items)
                    .cloned()
                    .collect(),
                folders: state
                    .folders
                    .values()
                    .filter(|folder| {
                        folder
                            .conversation_ids
                            .iter()
                            .any(|id| visible_conversation_ids.contains(id))
                    })
                    .take(max_items)
                    .cloned()
                    .collect(),
                invoices: state
                    .invoices
                    .values()
                    .filter(|invoice| visible_conversation_ids.contains(&invoice.conversation_id))
                    .take(max_items)
                    .cloned()
                    .collect(),
                orders: state
                    .orders
                    .values()
                    .filter(|order| {
                        &order.buyer_id == actor_id
                            || state
                                .invoices
                                .get(&order.invoice_id)
                                .is_some_and(|invoice| &invoice.seller_id == actor_id)
                    })
                    .take(max_items)
                    .cloned()
                    .collect(),
                stories: state
                    .stories
                    .values()
                    .filter(|story| {
                        (story.pinned_to_profile || story.expires_at_ms > server_time_ms)
                            && story.is_visible_to(actor_id, false, false)
                    })
                    .take(max_items)
                    .cloned()
                    .collect(),
                communities: visible_conversation_ids
                    .iter()
                    .filter_map(|id| state.communities.get(id))
                    .take(max_items)
                    .cloned()
                    .collect(),
                bots: state.bots.bots.values().take(max_items).cloned().collect(),
                bot_executions: state
                    .bots
                    .executions
                    .values()
                    .filter(|execution| {
                        &execution.bot_id == actor_id
                            || visible_actor_ids.contains(&execution.bot_id)
                    })
                    .take(max_items)
                    .cloned()
                    .collect(),
                mini_apps: state.mini_apps.values().take(max_items).cloned().collect(),
                next_cursor: Some(self.cursor.to_string()),
            },
        }
    }

    fn mark_direct_messages_delivered(
        &mut self,
        actor_id: &ActorId,
        server_time_ms: i64,
    ) -> Result<(), MessagingServiceError> {
        let pending = self
            .engine
            .state()
            .conversations
            .values()
            .filter(|conversation| {
                matches!(
                    conversation.kind,
                    ConversationKind::Direct | ConversationKind::Secret
                ) && conversation
                    .participants
                    .iter()
                    .any(|participant| &participant.actor_id == actor_id)
            })
            .flat_map(|conversation| {
                self.engine
                    .state()
                    .messages
                    .get(&conversation.id)
                    .into_iter()
                    .flat_map(|messages| messages.values())
                    .filter(|message| {
                        &message.sender_id != actor_id
                            && message.delivery_state == DeliveryState::Sent
                    })
                    .map(|message| (conversation.id.clone(), message.id.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return Ok(());
        }
        let mut events = Vec::new();
        for (conversation_id, message_id) in pending {
            events.extend(self.engine.execute(Command::SetDeliveryState {
                conversation_id,
                message_id,
                state: DeliveryState::Delivered,
            })?);
        }
        self.cursor = self.cursor.saturating_add(events.len() as u64);
        let responses = events
            .into_iter()
            .filter_map(|event| self.project_event(event, server_time_ms))
            .collect::<Vec<_>>();
        let journal = self.journal_entries(actor_id, &responses);
        self.persist_with_events(server_time_ms, &journal)?;
        Ok(())
    }

    fn project_command(
        &self,
        actor_id: &ActorId,
        command: ClientCommand,
        now_ms: i64,
    ) -> Vec<Command> {
        match command {
            ClientCommand::Sync { .. } => Vec::new(),
            ClientCommand::UpsertProfile { actor } => vec![Command::UpsertActor { actor }],
            ClientCommand::SetPresence { presence } => vec![Command::SetPresence {
                actor_id: actor_id.clone(),
                presence,
            }],
            ClientCommand::CreateConversation { conversation }
            | ClientCommand::UpdateConversation { conversation } => {
                vec![Command::UpsertConversation { conversation }]
            }
            ClientCommand::ArchiveConversation {
                conversation_id,
                archived,
            } => vec![Command::ArchiveConversation {
                conversation_id,
                archived,
            }],
            ClientCommand::PinConversation {
                conversation_id,
                pinned,
            } => vec![Command::PinConversation {
                conversation_id,
                pinned,
            }],
            ClientCommand::SetConversationNotifications {
                conversation_id,
                settings,
            } => vec![Command::SetConversationNotifications {
                conversation_id,
                settings,
            }],
            ClientCommand::UpsertFolder { folder } => vec![Command::UpsertFolder { folder }],
            ClientCommand::DeleteFolder { folder_id } => vec![Command::DeleteFolder { folder_id }],
            ClientCommand::SendMessage {
                conversation_id,
                client_message_id,
                content,
                reply_to_message_id,
                thread_root_message_id,
                scheduled_at_ms,
                silent,
                protected_content,
            } => {
                let message_id = stable_message_id(actor_id, &client_message_id);
                vec![
                    Command::QueueMessage {
                        conversation_id: conversation_id.clone(),
                        local_message_id: message_id.clone(),
                        client_message_id,
                        sender_id: actor_id.clone(),
                        content,
                        reply_to_message_id,
                        thread_root_message_id,
                        created_at_ms: now_ms,
                        scheduled_at_ms,
                        silent,
                        protected_content,
                    },
                    Command::AcknowledgeMessage {
                        conversation_id,
                        local_message_id: message_id.clone(),
                        server_message_id: message_id,
                        accepted_at_ms: now_ms,
                    },
                ]
            }
            ClientCommand::ForwardMessage {
                source_conversation_id,
                message_id,
                destination_conversation_id,
                client_message_id,
            } => vec![Command::ForwardMessage {
                source_conversation_id,
                message_id,
                destination_conversation_id,
                local_message_id: MessageId::new(format!("local:{}", client_message_id.0)),
                client_message_id,
                sender_id: actor_id.clone(),
                created_at_ms: now_ms,
            }],
            ClientCommand::EditMessage {
                conversation_id,
                message_id,
                content,
            } => vec![Command::EditMessage {
                conversation_id,
                message_id,
                content,
                edited_at_ms: now_ms,
            }],
            ClientCommand::DeleteMessages {
                conversation_id,
                message_ids,
                ..
            } => vec![Command::DeleteMessages {
                conversation_id,
                message_ids,
            }],
            ClientCommand::MarkRead {
                conversation_id,
                message_id,
            } => {
                let mut commands = vec![Command::MarkRead {
                    conversation_id: conversation_id.clone(),
                    actor_id: actor_id.clone(),
                    message_id: message_id.clone(),
                }];
                let should_mark_message_read = self
                    .engine
                    .state()
                    .conversations
                    .get(&conversation_id)
                    .is_some_and(|conversation| {
                        matches!(
                            conversation.kind,
                            ConversationKind::Direct | ConversationKind::Secret
                        )
                    })
                    && self
                        .engine
                        .state()
                        .messages
                        .get(&conversation_id)
                        .and_then(|messages| messages.get(&message_id))
                        .is_some_and(|message| &message.sender_id != actor_id);
                if should_mark_message_read {
                    commands.push(Command::SetDeliveryState {
                        conversation_id,
                        message_id,
                        state: DeliveryState::Read,
                    });
                }
                commands
            }
            ClientCommand::SetReaction {
                conversation_id,
                message_id,
                reaction,
            } => vec![Command::SetReaction {
                conversation_id,
                message_id,
                reaction,
            }],
            ClientCommand::PinMessage {
                conversation_id,
                message_id,
                pinned,
            } => vec![Command::PinMessage {
                conversation_id,
                message_id,
                pinned,
            }],
            ClientCommand::Search { .. }
            | ClientCommand::StartTyping { .. }
            | ClientCommand::StopTyping { .. } => Vec::new(),
            ClientCommand::CreateInvoice { invoice } => vec![Command::CreateInvoice { invoice }],
            ClientCommand::CheckoutInvoice {
                invoice_id,
                order_id,
                customer,
            } => vec![Command::CheckoutInvoice {
                invoice_id,
                order_id,
                buyer_id: actor_id.clone(),
                customer,
                created_at_ms: now_ms,
            }],
            ClientCommand::RefundOrder {
                order_id,
                request_id,
            } => vec![Command::RefundOrder {
                order_id,
                seller_id: actor_id.clone(),
                request_id,
                refunded_at_ms: now_ms,
            }],
            ClientCommand::WalletStatus => Vec::new(),
            ClientCommand::PublishStory { story } => vec![Command::PublishStory {
                actor_id: actor_id.clone(),
                story,
            }],
            ClientCommand::DeleteStory { story_id } => vec![Command::DeleteStory {
                actor_id: actor_id.clone(),
                story_id,
            }],
            ClientCommand::ViewStory { story_id } => vec![Command::ViewStory {
                actor_id: actor_id.clone(),
                story_id,
                viewed_at_ms: now_ms,
            }],
            ClientCommand::ReactStory { story_id, reaction } => vec![Command::ReactStory {
                actor_id: actor_id.clone(),
                story_id,
                reaction,
            }],
            ClientCommand::UpdateCommunity { community } => vec![Command::UpdateCommunity {
                actor_id: actor_id.clone(),
                community,
            }],
            ClientCommand::SetCommunityMember {
                conversation_id,
                member,
            } => vec![Command::SetCommunityMember {
                actor_id: actor_id.clone(),
                conversation_id,
                member,
            }],
            ClientCommand::CreateInviteLink { invite } => vec![Command::CreateInviteLink {
                actor_id: actor_id.clone(),
                invite,
            }],
            ClientCommand::RevokeInviteLink {
                conversation_id,
                invite_id,
            } => vec![Command::RevokeInviteLink {
                actor_id: actor_id.clone(),
                conversation_id,
                invite_id,
            }],
            ClientCommand::RequestCommunityJoin { request } => {
                vec![Command::RequestCommunityJoin {
                    actor_id: actor_id.clone(),
                    request,
                }]
            }
            ClientCommand::RespondCommunityJoin {
                conversation_id,
                requester_id,
                approved,
            } => vec![Command::RespondCommunityJoin {
                actor_id: actor_id.clone(),
                conversation_id,
                requester_id,
                approved,
                decided_at_ms: now_ms,
            }],
            ClientCommand::UpsertForumTopic { topic } => vec![Command::UpsertForumTopic {
                actor_id: actor_id.clone(),
                topic,
            }],
            ClientCommand::DeleteForumTopic {
                conversation_id,
                topic_id,
            } => vec![Command::DeleteForumTopic {
                actor_id: actor_id.clone(),
                conversation_id,
                topic_id,
            }],
            ClientCommand::RegisterBot { profile } => vec![Command::RegisterBot {
                actor_id: actor_id.clone(),
                profile,
            }],
            ClientCommand::InvokeBot { invocation } => vec![Command::BeginBotInvocation {
                actor_id: actor_id.clone(),
                invocation,
                created_at_ms: now_ms,
            }],
            ClientCommand::FinishBotExecution {
                execution_id,
                success,
                error,
            } => vec![Command::FinishBotExecution {
                actor_id: actor_id.clone(),
                execution_id,
                success,
                finished_at_ms: now_ms,
                error,
            }],
            ClientCommand::InstallMiniApp { manifest } => {
                vec![Command::InstallMiniApp { manifest }]
            }
            ClientCommand::GrantMiniApp { grant } => vec![Command::GrantMiniApp { grant }],
            ClientCommand::OpenMiniApp { session } => vec![Command::OpenMiniApp { session }],
            ClientCommand::MiniAppCall {
                session_id,
                request_id,
                request,
            } => vec![Command::MiniAppCall {
                session_id,
                request_id,
                request,
            }],
            ClientCommand::BeginBlobUpload { .. }
            | ClientCommand::AppendBlobChunk { .. }
            | ClientCommand::FinishBlobUpload { .. }
            | ClientCommand::DeleteBlob { .. } => Vec::new(),
        }
    }

    fn project_event(&self, event: Event, server_time_ms: i64) -> Option<ServerEnvelope> {
        let server_event = match event {
            Event::ActorUpserted { actor } => ServerEvent::ActorChanged { actor },
            Event::PresenceUpdated { actor_id, presence } => {
                ServerEvent::PresenceChanged { actor_id, presence }
            }
            Event::ConversationUpserted { conversation } => {
                ServerEvent::ConversationChanged { conversation }
            }
            Event::ConversationArchived {
                conversation_id, ..
            }
            | Event::ConversationPinned {
                conversation_id, ..
            }
            | Event::ConversationNotificationsUpdated {
                conversation_id, ..
            } => self
                .engine
                .state()
                .conversations
                .get(&conversation_id)
                .cloned()
                .map(|conversation| ServerEvent::ConversationChanged { conversation })?,
            Event::FolderUpserted { folder } => ServerEvent::FolderChanged { folder },
            Event::FolderDeleted { folder_id } => ServerEvent::FolderDeleted { folder_id },
            Event::MessageQueued { message } => ServerEvent::MessageAdded { message },
            Event::MessageAcknowledged {
                conversation_id,
                server_message_id,
                ..
            }
            | Event::DeliveryStateUpdated {
                conversation_id,
                message_id: server_message_id,
                ..
            }
            | Event::MessageEdited {
                conversation_id,
                message_id: server_message_id,
                ..
            }
            | Event::ReactionUpdated {
                conversation_id,
                message_id: server_message_id,
                ..
            }
            | Event::MessagePinned {
                conversation_id,
                message_id: server_message_id,
                ..
            } => self
                .engine
                .state()
                .messages
                .get(&conversation_id)
                .and_then(|messages| messages.get(&server_message_id))
                .cloned()
                .map(|message| ServerEvent::MessageChanged { message })?,
            Event::MessagesDeleted {
                conversation_id,
                message_ids,
            } => ServerEvent::MessagesDeleted {
                conversation_id,
                message_ids,
            },
            Event::ConversationRead {
                conversation_id,
                actor_id,
                message_id,
            } => ServerEvent::ReadChanged {
                conversation_id,
                actor_id,
                message_id,
            },
            Event::InvoiceCreated { invoice } => ServerEvent::InvoiceChanged { invoice },
            Event::OrderUpserted { order } => ServerEvent::OrderChanged { order },
            Event::WalletChanged { .. } => return None,
            Event::StoryChanged { story } => ServerEvent::StoryChanged { story },
            Event::StoryDeleted { story_id } => ServerEvent::StoryDeleted { story_id },
            Event::CommunityChanged { community } => ServerEvent::CommunityChanged { community },
            Event::BotRegistryChanged {
                profile, execution, ..
            } => ServerEvent::BotChanged { profile, execution },
            Event::MiniAppInstalled { manifest } => ServerEvent::MiniAppChanged { manifest },
            Event::MiniAppGrantUpdated { .. } => return None,
            Event::MiniAppOpened { session } => ServerEvent::MiniAppOpened { session },
            Event::MiniAppResponded {
                session_id,
                request_id,
                response,
            } => ServerEvent::MiniAppResult {
                session_id,
                request_id,
                response,
            },
        };
        Some(ServerEnvelope {
            protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
            cursor: Some(self.cursor.to_string()),
            server_time_ms,
            event: server_event,
        })
    }

    fn journal_entries(
        &self,
        initiator: &ActorId,
        responses: &[ServerEnvelope],
    ) -> Vec<JournalEntry> {
        responses
            .iter()
            .filter(|response| !matches!(&response.event, ServerEvent::SyncBatch { .. }))
            .map(|response| JournalEntry {
                envelope: response.clone(),
                audience: self.event_audience(initiator, &response.event),
            })
            .collect()
    }

    fn event_audience(&self, initiator: &ActorId, event: &ServerEvent) -> Vec<ActorId> {
        let mut audience = BTreeSet::from([initiator.clone()]);
        match event {
            ServerEvent::ActorChanged { actor } => {
                audience.insert(actor.id.clone());
                for conversation in
                    self.engine
                        .state()
                        .conversations
                        .values()
                        .filter(|conversation| {
                            conversation
                                .participants
                                .iter()
                                .any(|participant| participant.actor_id == actor.id)
                        })
                {
                    Self::extend_conversation_audience(&mut audience, conversation);
                }
            }
            ServerEvent::PresenceChanged { actor_id, .. } => {
                audience.insert(actor_id.clone());
                for conversation in
                    self.engine
                        .state()
                        .conversations
                        .values()
                        .filter(|conversation| {
                            conversation
                                .participants
                                .iter()
                                .any(|participant| &participant.actor_id == actor_id)
                        })
                {
                    Self::extend_conversation_audience(&mut audience, conversation);
                }
            }
            ServerEvent::ConversationChanged { conversation } => {
                Self::extend_conversation_audience(&mut audience, conversation);
            }
            ServerEvent::MessageAdded { message } | ServerEvent::MessageChanged { message } => {
                self.extend_conversation_id_audience(&mut audience, &message.conversation_id);
            }
            ServerEvent::MessagesDeleted {
                conversation_id, ..
            }
            | ServerEvent::ReadChanged {
                conversation_id, ..
            }
            | ServerEvent::TypingChanged {
                conversation_id, ..
            } => {
                self.extend_conversation_id_audience(&mut audience, conversation_id);
            }
            ServerEvent::InvoiceChanged { invoice } => {
                self.extend_conversation_id_audience(&mut audience, &invoice.conversation_id);
            }
            ServerEvent::OrderChanged { order } => {
                audience.insert(order.buyer_id.clone());
                if let Some(invoice) = self.engine.state().invoices.get(&order.invoice_id) {
                    audience.insert(invoice.seller_id.clone());
                }
            }
            ServerEvent::StoryChanged { story } => {
                audience.insert(story.owner_id.clone());
            }
            ServerEvent::CommunityChanged { community } => {
                self.extend_conversation_id_audience(&mut audience, &community.conversation_id);
            }
            ServerEvent::BotChanged { profile, execution } => {
                if let Some(profile) = profile {
                    audience.insert(profile.actor_id.clone());
                }
                if let Some(execution) = execution {
                    audience.insert(execution.bot_id.clone());
                }
            }
            ServerEvent::BotInvocationRequested { invocation } => {
                audience.insert(invocation.sender_id.clone());
                audience.insert(invocation.bot_id.clone());
                self.extend_conversation_id_audience(&mut audience, &invocation.conversation_id);
            }
            ServerEvent::MiniAppOpened { session } => {
                audience.insert(session.actor_id.clone());
            }
            ServerEvent::MiniAppResult { session_id, .. } => {
                if let Some(session) = self.engine.state().mini_app_sessions.get(session_id) {
                    audience.insert(session.actor_id.clone());
                }
            }
            ServerEvent::SyncBatch { .. }
            | ServerEvent::SearchResults { .. }
            | ServerEvent::FolderChanged { .. }
            | ServerEvent::FolderDeleted { .. }
            | ServerEvent::BlobUploadChanged { .. }
            | ServerEvent::BlobReady { .. }
            | ServerEvent::BlobDeleted { .. }
            | ServerEvent::WalletStatus { .. }
            | ServerEvent::StoryDeleted { .. }
            | ServerEvent::MiniAppChanged { .. }
            | ServerEvent::Error { .. } => {}
        }
        audience.into_iter().collect()
    }

    fn extend_conversation_id_audience(
        &self,
        audience: &mut BTreeSet<ActorId>,
        conversation_id: &ConversationId,
    ) {
        if let Some(conversation) = self.engine.state().conversations.get(conversation_id) {
            Self::extend_conversation_audience(audience, conversation);
        }
    }

    fn extend_conversation_audience(
        audience: &mut BTreeSet<ActorId>,
        conversation: &crate::conversation::Conversation,
    ) {
        audience.extend(
            conversation
                .participants
                .iter()
                .map(|participant| participant.actor_id.clone()),
        );
        if let Some(owner_id) = &conversation.owner_id {
            audience.insert(owner_id.clone());
        }
    }

    fn persist_with_events(
        &mut self,
        now_ms: i64,
        events: &[JournalEntry],
    ) -> Result<(), MessagingServiceError> {
        let snapshot = MessagingSnapshot::new(self.engine.state().clone(), self.cursor, now_ms);
        self.store.save_with_events(&snapshot, events)?;
        Ok(())
    }

    fn persist(&mut self, now_ms: i64) -> Result<(), MessagingServiceError> {
        let snapshot = MessagingSnapshot::new(self.engine.state().clone(), self.cursor, now_ms);
        self.store.save(&snapshot)?;
        Ok(())
    }
}
