use crate::actor::{Actor, ActorId, ActorKind, Participant, ParticipantRole, Presence};
use crate::bot::{BotExecution, BotInvocation, BotProfile, BotRegistry};
use crate::community::{
    CommunityError, CommunityMember, CommunityState, ForumTopicState, InviteLink, JoinRequest,
    MemberStatus,
};
use crate::conversation::{
    Conversation, ConversationDraft, ConversationFolder, ConversationId, NotificationSettings,
};
use crate::message::{
    ClientMessageId, DeliveryState, Message, MessageContent, MessageId, ReactionSummary,
};
use crate::miniapp::{
    MiniAppGrant, MiniAppManifest, MiniAppPermission, MiniAppRequest, MiniAppResponse,
    MiniAppSession,
};
use crate::payment::{CustomerInfo, Invoice, Money, PaymentOrder, PaymentStatus};
use crate::story::{Story, StoryError, StoryId};
use crate::wallet::{LedgerEntry, WalletAccountId, WalletError, WalletLedger};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum Command {
    UpsertActor {
        actor: Actor,
    },
    SetPresence {
        actor_id: ActorId,
        presence: Presence,
    },
    UpsertConversation {
        conversation: Conversation,
    },
    UpdateConversationInfo {
        conversation_id: ConversationId,
        title: String,
        description: Option<String>,
    },
    SetConversationParticipant {
        conversation_id: ConversationId,
        participant: Participant,
    },
    RemoveConversationParticipant {
        conversation_id: ConversationId,
        actor_id: ActorId,
    },
    ArchiveConversation {
        conversation_id: ConversationId,
        archived: bool,
    },
    PinConversation {
        conversation_id: ConversationId,
        pinned: bool,
    },
    SetMarkedUnread {
        conversation_id: ConversationId,
        actor_id: ActorId,
        marked_unread: bool,
    },
    SetDraft {
        draft: ConversationDraft,
    },
    SetConversationNotifications {
        conversation_id: ConversationId,
        settings: NotificationSettings,
    },
    UpsertFolder {
        folder: ConversationFolder,
    },
    DeleteFolder {
        folder_id: String,
    },
    QueueMessage {
        conversation_id: ConversationId,
        local_message_id: MessageId,
        client_message_id: ClientMessageId,
        sender_id: ActorId,
        content: MessageContent,
        reply_to_message_id: Option<MessageId>,
        thread_root_message_id: Option<MessageId>,
        created_at_ms: i64,
        scheduled_at_ms: Option<i64>,
        silent: bool,
        protected_content: bool,
    },
    ForwardMessage {
        source_conversation_id: ConversationId,
        message_id: MessageId,
        destination_conversation_id: ConversationId,
        local_message_id: MessageId,
        client_message_id: ClientMessageId,
        sender_id: ActorId,
        created_at_ms: i64,
    },
    AcknowledgeMessage {
        conversation_id: ConversationId,
        local_message_id: MessageId,
        server_message_id: MessageId,
        accepted_at_ms: i64,
    },
    SetDeliveryState {
        conversation_id: ConversationId,
        message_id: MessageId,
        state: DeliveryState,
    },
    EditMessage {
        conversation_id: ConversationId,
        message_id: MessageId,
        content: MessageContent,
        edited_at_ms: i64,
    },
    DeleteMessages {
        conversation_id: ConversationId,
        message_ids: Vec<MessageId>,
    },
    MarkRead {
        conversation_id: ConversationId,
        actor_id: ActorId,
        message_id: MessageId,
    },
    SetReaction {
        conversation_id: ConversationId,
        message_id: MessageId,
        reaction: ReactionSummary,
    },
    PinMessage {
        conversation_id: ConversationId,
        message_id: MessageId,
        pinned: bool,
    },
    VotePoll {
        conversation_id: ConversationId,
        message_id: MessageId,
        actor_id: ActorId,
        option_ids: Vec<String>,
    },
    CreateInvoice {
        invoice: Invoice,
    },
    UpsertOrder {
        order: PaymentOrder,
    },
    CheckoutInvoice {
        invoice_id: String,
        order_id: String,
        buyer_id: ActorId,
        customer: Option<CustomerInfo>,
        created_at_ms: i64,
    },
    RefundOrder {
        order_id: String,
        seller_id: ActorId,
        request_id: String,
        refunded_at_ms: i64,
    },
    CreditWalletSettlement {
        request_id: String,
        owner_id: ActorId,
        amount: Money,
        reference: Option<String>,
        settled_at_ms: i64,
    },
    PublishStory {
        actor_id: ActorId,
        story: Story,
    },
    DeleteStory {
        actor_id: ActorId,
        story_id: StoryId,
    },
    ViewStory {
        actor_id: ActorId,
        story_id: StoryId,
        viewed_at_ms: i64,
    },
    ReactStory {
        actor_id: ActorId,
        story_id: StoryId,
        reaction: Option<String>,
    },
    UpdateCommunity {
        actor_id: ActorId,
        community: CommunityState,
    },
    SetCommunityMember {
        actor_id: ActorId,
        conversation_id: ConversationId,
        member: CommunityMember,
    },
    CreateInviteLink {
        actor_id: ActorId,
        invite: InviteLink,
    },
    RevokeInviteLink {
        actor_id: ActorId,
        conversation_id: ConversationId,
        invite_id: String,
    },
    RequestCommunityJoin {
        actor_id: ActorId,
        request: JoinRequest,
    },
    RespondCommunityJoin {
        actor_id: ActorId,
        conversation_id: ConversationId,
        requester_id: ActorId,
        approved: bool,
        decided_at_ms: i64,
    },
    UpsertForumTopic {
        actor_id: ActorId,
        topic: ForumTopicState,
    },
    DeleteForumTopic {
        actor_id: ActorId,
        conversation_id: ConversationId,
        topic_id: String,
    },
    RegisterBot {
        actor_id: ActorId,
        profile: BotProfile,
    },
    BeginBotInvocation {
        actor_id: ActorId,
        invocation: BotInvocation,
        created_at_ms: i64,
    },
    FinishBotExecution {
        actor_id: ActorId,
        execution_id: String,
        success: bool,
        finished_at_ms: i64,
        error: Option<String>,
    },
    InstallMiniApp {
        manifest: MiniAppManifest,
    },
    GrantMiniApp {
        grant: MiniAppGrant,
    },
    OpenMiniApp {
        session: MiniAppSession,
    },
    MiniAppCall {
        session_id: String,
        request_id: String,
        request: MiniAppRequest,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum Event {
    ActorUpserted {
        actor: Actor,
    },
    PresenceUpdated {
        actor_id: ActorId,
        presence: Presence,
    },
    ConversationUpserted {
        conversation: Conversation,
    },
    ConversationInfoUpdated {
        conversation_id: ConversationId,
        title: String,
        description: Option<String>,
    },
    ConversationParticipantUpserted {
        conversation_id: ConversationId,
        participant: Participant,
    },
    ConversationParticipantRemoved {
        conversation_id: ConversationId,
        actor_id: ActorId,
    },
    ConversationArchived {
        conversation_id: ConversationId,
        archived: bool,
    },
    ConversationPinned {
        conversation_id: ConversationId,
        pinned: bool,
    },
    ConversationMarkedUnread {
        conversation_id: ConversationId,
        actor_id: ActorId,
        marked_unread: bool,
    },
    DraftChanged {
        draft: ConversationDraft,
    },
    ConversationNotificationsUpdated {
        conversation_id: ConversationId,
        settings: NotificationSettings,
    },
    FolderUpserted {
        folder: ConversationFolder,
    },
    FolderDeleted {
        folder_id: String,
    },
    MessageQueued {
        message: Message,
    },
    MessageAcknowledged {
        conversation_id: ConversationId,
        local_message_id: MessageId,
        server_message_id: MessageId,
        accepted_at_ms: i64,
    },
    DeliveryStateUpdated {
        conversation_id: ConversationId,
        message_id: MessageId,
        state: DeliveryState,
    },
    MessageEdited {
        conversation_id: ConversationId,
        message_id: MessageId,
        content: MessageContent,
        edited_at_ms: i64,
    },
    MessagesDeleted {
        conversation_id: ConversationId,
        message_ids: Vec<MessageId>,
    },
    ConversationRead {
        conversation_id: ConversationId,
        actor_id: ActorId,
        message_id: MessageId,
    },
    ReactionUpdated {
        conversation_id: ConversationId,
        message_id: MessageId,
        reaction: ReactionSummary,
    },
    MessagePinned {
        conversation_id: ConversationId,
        message_id: MessageId,
        pinned: bool,
    },
    PollVoteChanged {
        conversation_id: ConversationId,
        message_id: MessageId,
        actor_id: ActorId,
        option_ids: Vec<String>,
    },
    InvoiceCreated {
        invoice: Invoice,
    },
    OrderUpserted {
        order: PaymentOrder,
    },
    WalletChanged {
        wallet: WalletLedger,
        entry: LedgerEntry,
    },
    StoryChanged {
        story: Story,
    },
    StoryDeleted {
        story_id: StoryId,
    },
    CommunityChanged {
        community: CommunityState,
    },
    BotRegistryChanged {
        registry: BotRegistry,
        profile: Option<BotProfile>,
        execution: Option<BotExecution>,
    },
    MiniAppInstalled {
        manifest: MiniAppManifest,
    },
    MiniAppGrantUpdated {
        grant: MiniAppGrant,
    },
    MiniAppOpened {
        session: MiniAppSession,
    },
    MiniAppResponded {
        session_id: String,
        request_id: String,
        response: MiniAppResponse,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MessagingState {
    pub actors: BTreeMap<ActorId, Actor>,
    pub conversations: BTreeMap<ConversationId, Conversation>,
    pub folders: BTreeMap<String, ConversationFolder>,
    pub messages: BTreeMap<ConversationId, BTreeMap<MessageId, Message>>,
    pub read_cursors: BTreeMap<ConversationId, BTreeMap<ActorId, MessageId>>,
    pub poll_votes:
        BTreeMap<ConversationId, BTreeMap<MessageId, BTreeMap<ActorId, BTreeSet<String>>>>,
    pub marked_unread_by_actor: BTreeMap<ConversationId, BTreeSet<ActorId>>,
    pub drafts: BTreeMap<ConversationId, BTreeMap<ActorId, ConversationDraft>>,
    pub invoices: BTreeMap<String, Invoice>,
    pub orders: BTreeMap<String, PaymentOrder>,
    pub wallet: WalletLedger,
    pub stories: BTreeMap<StoryId, Story>,
    pub communities: BTreeMap<ConversationId, CommunityState>,
    pub bots: BotRegistry,
    pub mini_apps: BTreeMap<String, MiniAppManifest>,
    pub mini_app_grants: BTreeMap<(String, ActorId), MiniAppGrant>,
    pub mini_app_sessions: BTreeMap<String, MiniAppSession>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EngineError {
    #[error("actor identifier is invalid")]
    InvalidActor,
    #[error("actor {0:?} does not exist")]
    ActorNotFound(ActorId),
    #[error("conversation identifier is invalid")]
    InvalidConversation,
    #[error("conversation {0:?} does not exist")]
    ConversationNotFound(ConversationId),
    #[error("conversation participant data is invalid")]
    InvalidConversationParticipant,
    #[error("message {message_id:?} does not exist in conversation {conversation_id:?}")]
    MessageNotFound {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    #[error("message {message_id:?} already exists in conversation {conversation_id:?}")]
    DuplicateMessage {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    #[error("client message identifier is invalid")]
    InvalidClientMessageId,
    #[error("message id list must not be empty")]
    EmptyMessageList,
    #[error("poll vote is invalid")]
    InvalidPollVote,
    #[error("message is protected from forwarding")]
    ProtectedContent,
    #[error("actor {actor_id:?} is not a participant of conversation {conversation_id:?}")]
    SenderNotParticipant {
        conversation_id: ConversationId,
        actor_id: ActorId,
    },
    #[error("conversation {0:?} does not allow sending messages")]
    MessageSendPermissionDenied(ConversationId),
    #[error("conversation {0:?} does not allow sending media")]
    MediaSendPermissionDenied(ConversationId),
    #[error("conversation {0:?} does not allow sending polls")]
    PollSendPermissionDenied(ConversationId),
    #[error("secret conversations must contain exactly two participants")]
    InvalidSecretConversation,
    #[error("secret conversations only accept encrypted secret content or service messages")]
    SecretPlaintextRejected,
    #[error("secret content may only be sent inside a secret conversation")]
    SecretContentOutsideSecretConversation,
    #[error("secret message envelope identity does not match the conversation participants")]
    SecretEnvelopeMismatch,
    #[error("story is invalid")]
    InvalidStory,
    #[error("story {0:?} does not exist")]
    StoryNotFound(StoryId),
    #[error("only the story owner may modify story {0:?}")]
    StoryPermissionDenied(StoryId),
    #[error(transparent)]
    Story(#[from] StoryError),
    #[error("community {0:?} does not exist")]
    CommunityNotFound(ConversationId),
    #[error("community administration permission denied")]
    CommunityPermissionDenied,
    #[error("community {0:?} member is not allowed to send messages")]
    CommunitySendRestricted(ConversationId),
    #[error("community {0:?} member is not allowed to send media")]
    CommunityMediaRestricted(ConversationId),
    #[error("community {0:?} member is not allowed to send polls")]
    CommunityPollRestricted(ConversationId),
    #[error(transparent)]
    Community(#[from] CommunityError),
    #[error("bot operation permission denied")]
    BotPermissionDenied,
    #[error("bot operation failed: {0}")]
    Bot(String),
    #[error("invoice is invalid")]
    InvalidInvoice,
    #[error("invoice {0} does not exist")]
    InvoiceNotFound(String),
    #[error("payment order {0} does not exist")]
    OrderNotFound(String),
    #[error("payment invoice {0} has expired")]
    InvoiceExpired(String),
    #[error("payment order {0} conflicts with an existing order")]
    OrderConflict(String),
    #[error("only the invoice seller may refund order {0}")]
    RefundForbidden(String),
    #[error("payment order {0} is not refundable in its current state")]
    OrderNotRefundable(String),
    #[error(transparent)]
    Wallet(#[from] WalletError),
    #[error("Mini App {0} is not installed")]
    MiniAppNotFound(String),
    #[error("Mini App session {0} does not exist")]
    MiniAppSessionNotFound(String),
    #[error("Mini App permission {0:?} was not granted")]
    MiniAppPermissionDenied(MiniAppPermission),
    #[error("Mini App request cannot be completed by the pure domain engine")]
    MiniAppHostActionRequired,
}

#[derive(Debug, Clone, Default)]
pub struct MessagingEngine {
    state: MessagingState,
}

#[derive(Debug, Clone, Copy)]
enum CommunityAdminAction {
    ChangeInfo,
    InviteMembers,
    BanMembers,
    ManageTopics,
    AddAdmins,
}

fn is_community_owner(community: &CommunityState, actor_id: &ActorId) -> bool {
    community
        .members
        .get(actor_id)
        .is_some_and(|member| matches!(member.status, MemberStatus::Owner))
}

fn require_community_admin(
    community: &CommunityState,
    actor_id: &ActorId,
    action: CommunityAdminAction,
) -> Result<(), EngineError> {
    let allowed = community.members.get(actor_id).is_some_and(|member| {
        if matches!(member.status, MemberStatus::Owner) {
            return true;
        }
        if !matches!(member.status, MemberStatus::Administrator) {
            return false;
        }
        match action {
            CommunityAdminAction::ChangeInfo => member.admin_rights.change_info,
            CommunityAdminAction::InviteMembers => member.admin_rights.invite_members,
            CommunityAdminAction::BanMembers => member.admin_rights.ban_members,
            CommunityAdminAction::ManageTopics => member.admin_rights.manage_topics,
            CommunityAdminAction::AddAdmins => member.admin_rights.add_admins,
        }
    });
    if allowed {
        Ok(())
    } else {
        Err(EngineError::CommunityPermissionDenied)
    }
}

fn message_content_uses_media(content: &MessageContent) -> bool {
    matches!(
        content,
        MessageContent::Photo { .. }
            | MessageContent::Video { .. }
            | MessageContent::Animation { .. }
            | MessageContent::Audio { .. }
            | MessageContent::Voice { .. }
            | MessageContent::VideoNote { .. }
            | MessageContent::Document { .. }
            | MessageContent::Sticker { .. }
    )
}

fn wallet_account_id(actor_id: &ActorId) -> WalletAccountId {
    WalletAccountId(format!("wallet:{}", actor_id.0))
}

fn ensure_wallet_account(
    wallet: &mut WalletLedger,
    account_id: &WalletAccountId,
    owner_id: &ActorId,
    now_ms: i64,
) -> Result<(), WalletError> {
    if wallet.accounts.contains_key(account_id) {
        return Ok(());
    }
    wallet.create_account(account_id.clone(), owner_id.clone(), now_ms)
}

impl MessagingEngine {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn from_state(state: MessagingState) -> Self {
        Self { state }
    }
    pub fn state(&self) -> &MessagingState {
        &self.state
    }
    pub fn into_state(self) -> MessagingState {
        self.state
    }

    pub fn execute(&mut self, command: Command) -> Result<Vec<Event>, EngineError> {
        let events = self.decide(command)?;
        for event in events.iter().cloned() {
            self.apply(event);
        }
        Ok(events)
    }

    pub fn decide(&self, command: Command) -> Result<Vec<Event>, EngineError> {
        match command {
            Command::UpsertActor { actor } => {
                if !actor.id.is_valid() || actor.display_name.trim().is_empty() {
                    return Err(EngineError::InvalidActor);
                }
                Ok(vec![Event::ActorUpserted { actor }])
            }
            Command::SetPresence { actor_id, presence } => {
                self.require_actor(&actor_id)?;
                Ok(vec![Event::PresenceUpdated { actor_id, presence }])
            }
            Command::UpsertConversation { conversation } => {
                if !conversation.id.is_valid() || conversation.title.trim().is_empty() {
                    return Err(EngineError::InvalidConversation);
                }
                for participant in &conversation.participants {
                    self.require_actor(&participant.actor_id)?;
                }
                if matches!(
                    conversation.kind,
                    crate::conversation::ConversationKind::Secret
                ) && conversation.participants.len() != 2
                {
                    return Err(EngineError::InvalidSecretConversation);
                }
                Ok(vec![Event::ConversationUpserted { conversation }])
            }
            Command::UpdateConversationInfo {
                conversation_id,
                title,
                description,
            } => {
                self.require_conversation(&conversation_id)?;
                if title.trim().is_empty() || title.trim().len() > 200 {
                    return Err(EngineError::InvalidConversationParticipant);
                }
                Ok(vec![Event::ConversationInfoUpdated {
                    conversation_id,
                    title: title.trim().to_string(),
                    description: description
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty()),
                }])
            }
            Command::SetConversationParticipant {
                conversation_id,
                participant,
            } => {
                let conversation = self.require_conversation(&conversation_id)?;
                self.require_actor(&participant.actor_id)?;
                if conversation.owner_id.as_ref() == Some(&participant.actor_id)
                    && !matches!(participant.role, ParticipantRole::Owner)
                {
                    return Err(EngineError::InvalidConversationParticipant);
                }
                Ok(vec![Event::ConversationParticipantUpserted {
                    conversation_id,
                    participant,
                }])
            }
            Command::RemoveConversationParticipant {
                conversation_id,
                actor_id,
            } => {
                let conversation = self.require_conversation(&conversation_id)?;
                if conversation.owner_id.as_ref() == Some(&actor_id) {
                    return Err(EngineError::InvalidConversationParticipant);
                }
                Ok(vec![Event::ConversationParticipantRemoved {
                    conversation_id,
                    actor_id,
                }])
            }
            Command::ArchiveConversation {
                conversation_id,
                archived,
            } => {
                self.require_conversation(&conversation_id)?;
                Ok(vec![Event::ConversationArchived {
                    conversation_id,
                    archived,
                }])
            }
            Command::PinConversation {
                conversation_id,
                pinned,
            } => {
                self.require_conversation(&conversation_id)?;
                Ok(vec![Event::ConversationPinned {
                    conversation_id,
                    pinned,
                }])
            }
            Command::SetMarkedUnread {
                conversation_id,
                actor_id,
                marked_unread,
            } => {
                self.require_conversation(&conversation_id)?;
                self.require_actor(&actor_id)?;
                Ok(vec![Event::ConversationMarkedUnread {
                    conversation_id,
                    actor_id,
                    marked_unread,
                }])
            }
            Command::SetDraft { draft } => {
                self.require_conversation(&draft.conversation_id)?;
                self.require_actor(&draft.actor_id)?;
                Ok(vec![Event::DraftChanged { draft }])
            }
            Command::SetConversationNotifications {
                conversation_id,
                settings,
            } => {
                self.require_conversation(&conversation_id)?;
                Ok(vec![Event::ConversationNotificationsUpdated {
                    conversation_id,
                    settings,
                }])
            }
            Command::UpsertFolder { folder } => Ok(vec![Event::FolderUpserted { folder }]),
            Command::DeleteFolder { folder_id } => Ok(vec![Event::FolderDeleted { folder_id }]),
            Command::QueueMessage {
                conversation_id,
                local_message_id,
                client_message_id,
                sender_id,
                content,
                reply_to_message_id,
                thread_root_message_id,
                created_at_ms,
                scheduled_at_ms,
                silent,
                protected_content,
            } => {
                let conversation = self.require_conversation(&conversation_id)?;
                self.require_actor(&sender_id)?;
                let sender_is_participant = conversation
                    .participants
                    .iter()
                    .any(|participant| participant.actor_id == sender_id)
                    || conversation.owner_id.as_ref() == Some(&sender_id);
                if !sender_is_participant {
                    return Err(EngineError::SenderNotParticipant {
                        conversation_id: conversation_id.clone(),
                        actor_id: sender_id.clone(),
                    });
                }
                if !conversation.permissions.can_send_messages {
                    return Err(EngineError::MessageSendPermissionDenied(
                        conversation_id.clone(),
                    ));
                }
                if message_content_uses_media(&content) && !conversation.permissions.can_send_media
                {
                    return Err(EngineError::MediaSendPermissionDenied(
                        conversation_id.clone(),
                    ));
                }
                if matches!(&content, MessageContent::Poll { .. })
                    && !conversation.permissions.can_send_polls
                {
                    return Err(EngineError::PollSendPermissionDenied(
                        conversation_id.clone(),
                    ));
                }
                if let Some(member) = self
                    .state
                    .communities
                    .get(&conversation_id)
                    .and_then(|community| community.members.get(&sender_id))
                {
                    if matches!(member.status, MemberStatus::Left | MemberStatus::Banned)
                        || (matches!(member.status, MemberStatus::Restricted)
                            && member.restrictions.send_messages)
                    {
                        return Err(EngineError::CommunitySendRestricted(
                            conversation_id.clone(),
                        ));
                    }
                    if message_content_uses_media(&content)
                        && matches!(member.status, MemberStatus::Restricted)
                        && member.restrictions.send_media
                    {
                        return Err(EngineError::CommunityMediaRestricted(
                            conversation_id.clone(),
                        ));
                    }
                    if matches!(&content, MessageContent::Poll { .. })
                        && matches!(member.status, MemberStatus::Restricted)
                        && member.restrictions.send_polls
                    {
                        return Err(EngineError::CommunityPollRestricted(
                            conversation_id.clone(),
                        ));
                    }
                }
                let is_secret_conversation = matches!(
                    conversation.kind,
                    crate::conversation::ConversationKind::Secret
                );
                let is_secret_content = matches!(&content, MessageContent::Secret { .. });
                let is_service_content = matches!(&content, MessageContent::Service { .. });
                if is_secret_conversation && !is_secret_content && !is_service_content {
                    return Err(EngineError::SecretPlaintextRejected);
                }
                if !is_secret_conversation && is_secret_content {
                    return Err(EngineError::SecretContentOutsideSecretConversation);
                }
                if let MessageContent::Secret { envelope } = &content {
                    let participant_ids = conversation
                        .participants
                        .iter()
                        .map(|participant| &participant.actor_id)
                        .collect::<Vec<_>>();
                    if envelope.conversation_id != conversation_id
                        || envelope.sender_id != sender_id
                        || !participant_ids.contains(&&envelope.sender_id)
                        || !participant_ids.contains(&&envelope.recipient_id)
                        || envelope.sender_id == envelope.recipient_id
                    {
                        return Err(EngineError::SecretEnvelopeMismatch);
                    }
                }
                if client_message_id.0.trim().is_empty() || client_message_id.0.len() > 200 {
                    return Err(EngineError::InvalidClientMessageId);
                }
                if self
                    .state
                    .messages
                    .get(&conversation_id)
                    .is_some_and(|messages| messages.contains_key(&local_message_id))
                {
                    return Err(EngineError::DuplicateMessage {
                        conversation_id,
                        message_id: local_message_id,
                    });
                }
                let message = Message {
                    id: local_message_id,
                    conversation_id,
                    sender_id,
                    content,
                    reply_to_message_id,
                    thread_root_message_id,
                    forward_origin: None,
                    reply_markup: None,
                    reactions: Vec::new(),
                    delivery_state: DeliveryState::Pending { client_message_id },
                    created_at_ms,
                    edited_at_ms: None,
                    scheduled_at_ms,
                    silent,
                    protected_content: protected_content || is_secret_content,
                    pinned: false,
                    deleted: false,
                };
                Ok(vec![Event::MessageQueued { message }])
            }
            Command::ForwardMessage {
                source_conversation_id,
                message_id,
                destination_conversation_id,
                local_message_id,
                client_message_id,
                sender_id,
                created_at_ms,
            } => {
                self.require_conversation(&destination_conversation_id)?;
                self.require_actor(&sender_id)?;
                if client_message_id.0.trim().is_empty() || client_message_id.0.len() > 200 {
                    return Err(EngineError::InvalidClientMessageId);
                }
                if self
                    .state
                    .messages
                    .get(&destination_conversation_id)
                    .is_some_and(|messages| messages.contains_key(&local_message_id))
                {
                    return Err(EngineError::DuplicateMessage {
                        conversation_id: destination_conversation_id,
                        message_id: local_message_id,
                    });
                }
                let original = self
                    .require_message(&source_conversation_id, &message_id)?
                    .clone();
                if original.deleted {
                    return Err(EngineError::MessageNotFound {
                        conversation_id: source_conversation_id,
                        message_id,
                    });
                }
                if original.protected_content {
                    return Err(EngineError::ProtectedContent);
                }
                let forward_origin = original
                    .forward_origin
                    .clone()
                    .unwrap_or_else(|| format!("{}:{}", original.conversation_id.0, original.id.0));
                let message = Message {
                    id: local_message_id,
                    conversation_id: destination_conversation_id,
                    sender_id,
                    content: original.content,
                    reply_to_message_id: None,
                    thread_root_message_id: None,
                    forward_origin: Some(forward_origin),
                    reply_markup: original.reply_markup,
                    reactions: Vec::new(),
                    delivery_state: DeliveryState::Pending { client_message_id },
                    created_at_ms,
                    edited_at_ms: None,
                    scheduled_at_ms: None,
                    silent: false,
                    protected_content: original.protected_content,
                    pinned: false,
                    deleted: false,
                };
                Ok(vec![Event::MessageQueued { message }])
            }
            Command::AcknowledgeMessage {
                conversation_id,
                local_message_id,
                server_message_id,
                accepted_at_ms,
            } => {
                self.require_message(&conversation_id, &local_message_id)?;
                if local_message_id != server_message_id
                    && self
                        .state
                        .messages
                        .get(&conversation_id)
                        .is_some_and(|messages| messages.contains_key(&server_message_id))
                {
                    return Err(EngineError::DuplicateMessage {
                        conversation_id,
                        message_id: server_message_id,
                    });
                }
                Ok(vec![Event::MessageAcknowledged {
                    conversation_id,
                    local_message_id,
                    server_message_id,
                    accepted_at_ms,
                }])
            }
            Command::SetDeliveryState {
                conversation_id,
                message_id,
                state,
            } => {
                self.require_message(&conversation_id, &message_id)?;
                Ok(vec![Event::DeliveryStateUpdated {
                    conversation_id,
                    message_id,
                    state,
                }])
            }
            Command::EditMessage {
                conversation_id,
                message_id,
                content,
                edited_at_ms,
            } => {
                self.require_message(&conversation_id, &message_id)?;
                Ok(vec![Event::MessageEdited {
                    conversation_id,
                    message_id,
                    content,
                    edited_at_ms,
                }])
            }
            Command::DeleteMessages {
                conversation_id,
                message_ids,
            } => {
                let unique: BTreeSet<_> = message_ids.into_iter().collect();
                if unique.is_empty() {
                    return Err(EngineError::EmptyMessageList);
                }
                for id in &unique {
                    self.require_message(&conversation_id, id)?;
                }
                Ok(vec![Event::MessagesDeleted {
                    conversation_id,
                    message_ids: unique.into_iter().collect(),
                }])
            }
            Command::MarkRead {
                conversation_id,
                actor_id,
                message_id,
            } => {
                self.require_actor(&actor_id)?;
                self.require_message(&conversation_id, &message_id)?;
                Ok(vec![Event::ConversationRead {
                    conversation_id,
                    actor_id,
                    message_id,
                }])
            }
            Command::SetReaction {
                conversation_id,
                message_id,
                reaction,
            } => {
                self.require_message(&conversation_id, &message_id)?;
                if reaction.reaction.trim().is_empty() {
                    return Err(EngineError::InvalidClientMessageId);
                }
                Ok(vec![Event::ReactionUpdated {
                    conversation_id,
                    message_id,
                    reaction,
                }])
            }
            Command::PinMessage {
                conversation_id,
                message_id,
                pinned,
            } => {
                self.require_message(&conversation_id, &message_id)?;
                Ok(vec![Event::MessagePinned {
                    conversation_id,
                    message_id,
                    pinned,
                }])
            }
            Command::VotePoll {
                conversation_id,
                message_id,
                actor_id,
                option_ids,
            } => {
                self.require_actor(&actor_id)?;
                let message = self.require_message(&conversation_id, &message_id)?;
                let (valid_ids, multiple_answers) = match &message.content {
                    MessageContent::Poll {
                        options,
                        multiple_answers,
                        ..
                    } => (
                        options
                            .iter()
                            .map(|option| option.id.clone())
                            .collect::<BTreeSet<_>>(),
                        *multiple_answers,
                    ),
                    _ => return Err(EngineError::InvalidPollVote),
                };
                let selected = option_ids.into_iter().collect::<BTreeSet<_>>();
                if (!multiple_answers && selected.len() > 1)
                    || selected.iter().any(|id| !valid_ids.contains(id))
                {
                    return Err(EngineError::InvalidPollVote);
                }
                Ok(vec![Event::PollVoteChanged {
                    conversation_id,
                    message_id,
                    actor_id,
                    option_ids: selected.into_iter().collect(),
                }])
            }
            Command::CreateInvoice { invoice } => {
                if !invoice.is_valid() {
                    return Err(EngineError::InvalidInvoice);
                }
                self.require_actor(&invoice.seller_id)?;
                self.require_conversation(&invoice.conversation_id)?;
                Ok(vec![Event::InvoiceCreated { invoice }])
            }
            Command::UpsertOrder { order } => {
                if !self.state.invoices.contains_key(&order.invoice_id) {
                    return Err(EngineError::InvoiceNotFound(order.invoice_id));
                }
                self.require_actor(&order.buyer_id)?;
                Ok(vec![Event::OrderUpserted { order }])
            }
            Command::CheckoutInvoice {
                invoice_id,
                order_id,
                buyer_id,
                customer,
                created_at_ms,
            } => {
                let invoice = self
                    .state
                    .invoices
                    .get(&invoice_id)
                    .cloned()
                    .ok_or_else(|| EngineError::InvoiceNotFound(invoice_id.clone()))?;
                self.require_actor(&buyer_id)?;
                if invoice
                    .expires_at_ms
                    .is_some_and(|expires_at_ms| expires_at_ms <= created_at_ms)
                {
                    return Err(EngineError::InvoiceExpired(invoice_id));
                }
                if let Some(existing) = self.state.orders.get(&order_id) {
                    if existing.invoice_id == invoice.id && existing.buyer_id == buyer_id {
                        return Ok(vec![Event::OrderUpserted {
                            order: existing.clone(),
                        }]);
                    }
                    return Err(EngineError::OrderConflict(order_id));
                }
                let amount_minor = invoice
                    .checked_total_minor()
                    .ok_or(EngineError::InvalidInvoice)?;
                if amount_minor <= 0 {
                    return Err(EngineError::InvalidInvoice);
                }
                let buyer_account = wallet_account_id(&buyer_id);
                let seller_account = wallet_account_id(&invoice.seller_id);
                let mut wallet = self.state.wallet.clone();
                ensure_wallet_account(&mut wallet, &buyer_account, &buyer_id, created_at_ms)?;
                ensure_wallet_account(
                    &mut wallet,
                    &seller_account,
                    &invoice.seller_id,
                    created_at_ms,
                )?;
                let entry = wallet.transfer(
                    format!("checkout:{order_id}"),
                    &buyer_account,
                    &seller_account,
                    Money::new(&invoice.currency, amount_minor),
                    Some(invoice.id.clone()),
                    created_at_ms,
                )?;
                let order = PaymentOrder {
                    id: order_id,
                    invoice_id: invoice.id,
                    buyer_id,
                    status: PaymentStatus::Paid,
                    amount: Money::new(&invoice.currency, amount_minor),
                    customer,
                    provider_payment_id: Some(entry.id.clone()),
                    provider_receipt_url: Some(format!("fabushi://payments/receipts/{}", entry.id)),
                    created_at_ms,
                    updated_at_ms: created_at_ms,
                };
                Ok(vec![
                    Event::WalletChanged { wallet, entry },
                    Event::OrderUpserted { order },
                ])
            }
            Command::RefundOrder {
                order_id,
                seller_id,
                request_id,
                refunded_at_ms,
            } => {
                let mut order = self
                    .state
                    .orders
                    .get(&order_id)
                    .cloned()
                    .ok_or_else(|| EngineError::OrderNotFound(order_id.clone()))?;
                let invoice = self
                    .state
                    .invoices
                    .get(&order.invoice_id)
                    .cloned()
                    .ok_or_else(|| EngineError::InvoiceNotFound(order.invoice_id.clone()))?;
                if invoice.seller_id != seller_id {
                    return Err(EngineError::RefundForbidden(order_id));
                }
                if order.status == PaymentStatus::Refunded {
                    return Ok(vec![Event::OrderUpserted { order }]);
                }
                if order.status != PaymentStatus::Paid {
                    return Err(EngineError::OrderNotRefundable(order_id));
                }
                let original_entry_id = order
                    .provider_payment_id
                    .clone()
                    .ok_or_else(|| EngineError::OrderNotRefundable(order.id.clone()))?;
                let mut wallet = self.state.wallet.clone();
                let entry =
                    wallet.refund_transfer(request_id, &original_entry_id, refunded_at_ms)?;
                order.status = PaymentStatus::Refunded;
                order.updated_at_ms = refunded_at_ms;
                Ok(vec![
                    Event::WalletChanged { wallet, entry },
                    Event::OrderUpserted { order },
                ])
            }
            Command::CreditWalletSettlement {
                request_id,
                owner_id,
                amount,
                reference,
                settled_at_ms,
            } => {
                self.require_actor(&owner_id)?;
                let account_id = wallet_account_id(&owner_id);
                let mut wallet = self.state.wallet.clone();
                ensure_wallet_account(&mut wallet, &account_id, &owner_id, settled_at_ms)?;
                let entry =
                    wallet.credit(request_id, &account_id, amount, reference, settled_at_ms)?;
                Ok(vec![Event::WalletChanged { wallet, entry }])
            }
            Command::PublishStory { actor_id, story } => {
                self.require_actor(&actor_id)?;
                if story.owner_id != actor_id
                    || story.id.0.trim().is_empty()
                    || story.media.id.trim().is_empty()
                    || story.expires_at_ms <= story.created_at_ms
                {
                    return Err(EngineError::InvalidStory);
                }
                Ok(vec![Event::StoryChanged { story }])
            }
            Command::DeleteStory { actor_id, story_id } => {
                let story = self
                    .state
                    .stories
                    .get(&story_id)
                    .ok_or_else(|| EngineError::StoryNotFound(story_id.clone()))?;
                if story.owner_id != actor_id {
                    return Err(EngineError::StoryPermissionDenied(story_id));
                }
                Ok(vec![Event::StoryDeleted { story_id }])
            }
            Command::ViewStory {
                actor_id,
                story_id,
                viewed_at_ms,
            } => {
                self.require_actor(&actor_id)?;
                let mut story = self
                    .state
                    .stories
                    .get(&story_id)
                    .cloned()
                    .ok_or_else(|| EngineError::StoryNotFound(story_id.clone()))?;
                if !story.is_visible_to(&actor_id, false, false) {
                    return Err(EngineError::StoryPermissionDenied(story_id));
                }
                story.record_view(actor_id, viewed_at_ms)?;
                Ok(vec![Event::StoryChanged { story }])
            }
            Command::ReactStory {
                actor_id,
                story_id,
                reaction,
            } => {
                let mut story = self
                    .state
                    .stories
                    .get(&story_id)
                    .cloned()
                    .ok_or_else(|| EngineError::StoryNotFound(story_id.clone()))?;
                if !story.is_visible_to(&actor_id, false, false) {
                    return Err(EngineError::StoryPermissionDenied(story_id));
                }
                story.react(&actor_id, reaction)?;
                Ok(vec![Event::StoryChanged { story }])
            }
            Command::UpdateCommunity {
                actor_id,
                community,
            } => {
                let conversation = self.require_conversation(&community.conversation_id)?;
                if !matches!(
                    conversation.kind,
                    crate::conversation::ConversationKind::Group
                        | crate::conversation::ConversationKind::Channel
                ) {
                    return Err(EngineError::CommunityPermissionDenied);
                }
                if let Some(existing) = self.state.communities.get(&community.conversation_id) {
                    require_community_admin(existing, &actor_id, CommunityAdminAction::ChangeInfo)?;
                } else if conversation.owner_id.as_ref() != Some(&actor_id)
                    && !conversation.participants.iter().any(|participant| {
                        participant.actor_id == actor_id
                            && matches!(participant.role, crate::actor::ParticipantRole::Owner)
                    })
                {
                    return Err(EngineError::CommunityPermissionDenied);
                }
                Ok(vec![Event::CommunityChanged { community }])
            }
            Command::SetCommunityMember {
                actor_id,
                conversation_id,
                member,
            } => {
                self.require_actor(&member.actor_id)?;
                let mut community = self
                    .state
                    .communities
                    .get(&conversation_id)
                    .cloned()
                    .ok_or_else(|| EngineError::CommunityNotFound(conversation_id.clone()))?;
                let caller_is_owner = is_community_owner(&community, &actor_id);
                let target_is_owner = community
                    .members
                    .get(&member.actor_id)
                    .is_some_and(|existing| matches!(existing.status, MemberStatus::Owner));
                if (target_is_owner || matches!(member.status, MemberStatus::Owner))
                    && !caller_is_owner
                {
                    return Err(EngineError::CommunityPermissionDenied);
                }
                let action = match member.status {
                    MemberStatus::Owner | MemberStatus::Administrator => {
                        CommunityAdminAction::AddAdmins
                    }
                    MemberStatus::Restricted | MemberStatus::Left | MemberStatus::Banned => {
                        CommunityAdminAction::BanMembers
                    }
                    MemberStatus::Member => CommunityAdminAction::InviteMembers,
                };
                require_community_admin(&community, &actor_id, action)?;
                community.upsert_member(member);
                Ok(vec![Event::CommunityChanged { community }])
            }
            Command::CreateInviteLink { actor_id, invite } => {
                let mut community = self
                    .state
                    .communities
                    .get(&invite.conversation_id)
                    .cloned()
                    .ok_or_else(|| {
                        EngineError::CommunityNotFound(invite.conversation_id.clone())
                    })?;
                require_community_admin(
                    &community,
                    &actor_id,
                    CommunityAdminAction::InviteMembers,
                )?;
                if invite.creator_id != actor_id
                    || invite.id.trim().is_empty()
                    || invite.token.trim().is_empty()
                    || invite.revoked
                {
                    return Err(EngineError::CommunityPermissionDenied);
                }
                community.invite_links.insert(invite.id.clone(), invite);
                Ok(vec![Event::CommunityChanged { community }])
            }
            Command::RevokeInviteLink {
                actor_id,
                conversation_id,
                invite_id,
            } => {
                let mut community = self
                    .state
                    .communities
                    .get(&conversation_id)
                    .cloned()
                    .ok_or_else(|| EngineError::CommunityNotFound(conversation_id.clone()))?;
                require_community_admin(
                    &community,
                    &actor_id,
                    CommunityAdminAction::InviteMembers,
                )?;
                community.revoke_invite(&invite_id)?;
                Ok(vec![Event::CommunityChanged { community }])
            }
            Command::RequestCommunityJoin { actor_id, request } => {
                if request.actor_id != actor_id {
                    return Err(EngineError::CommunityPermissionDenied);
                }
                self.require_actor(&actor_id)?;
                self.require_conversation(&request.conversation_id)?;
                let mut community = self
                    .state
                    .communities
                    .get(&request.conversation_id)
                    .cloned()
                    .unwrap_or_else(|| CommunityState::new(request.conversation_id.clone()));
                community.request_join(request);
                Ok(vec![Event::CommunityChanged { community }])
            }
            Command::RespondCommunityJoin {
                actor_id,
                conversation_id,
                requester_id,
                approved,
                decided_at_ms,
            } => {
                let mut community = self
                    .state
                    .communities
                    .get(&conversation_id)
                    .cloned()
                    .ok_or_else(|| EngineError::CommunityNotFound(conversation_id.clone()))?;
                require_community_admin(
                    &community,
                    &actor_id,
                    CommunityAdminAction::InviteMembers,
                )?;
                if approved {
                    community.approve_join(&requester_id, &actor_id, decided_at_ms)?;
                } else {
                    community
                        .pending_join_requests
                        .remove(&requester_id)
                        .ok_or_else(|| CommunityError::JoinRequestNotFound(requester_id.clone()))?;
                }
                Ok(vec![Event::CommunityChanged { community }])
            }
            Command::UpsertForumTopic { actor_id, topic } => {
                let mut community = self
                    .state
                    .communities
                    .get(&topic.conversation_id)
                    .cloned()
                    .ok_or_else(|| EngineError::CommunityNotFound(topic.conversation_id.clone()))?;
                require_community_admin(&community, &actor_id, CommunityAdminAction::ManageTopics)?;
                if topic.id.trim().is_empty() {
                    return Err(EngineError::CommunityPermissionDenied);
                }
                community.topics.insert(topic.id.clone(), topic);
                Ok(vec![Event::CommunityChanged { community }])
            }
            Command::DeleteForumTopic {
                actor_id,
                conversation_id,
                topic_id,
            } => {
                let mut community = self
                    .state
                    .communities
                    .get(&conversation_id)
                    .cloned()
                    .ok_or_else(|| EngineError::CommunityNotFound(conversation_id.clone()))?;
                require_community_admin(&community, &actor_id, CommunityAdminAction::ManageTopics)?;
                community.topics.remove(&topic_id);
                Ok(vec![Event::CommunityChanged { community }])
            }
            Command::RegisterBot { actor_id, profile } => {
                let actor = self.require_actor(&actor_id)?;
                if profile.actor_id != actor_id
                    || !matches!(actor.kind, ActorKind::Bot | ActorKind::Assistant)
                {
                    return Err(EngineError::BotPermissionDenied);
                }
                let mut registry = self.state.bots.clone();
                registry
                    .register(profile.clone())
                    .map_err(|error| EngineError::Bot(error.to_string()))?;
                Ok(vec![Event::BotRegistryChanged {
                    registry,
                    profile: Some(profile),
                    execution: None,
                }])
            }
            Command::BeginBotInvocation {
                actor_id,
                invocation,
                created_at_ms,
            } => {
                if invocation.sender_id != actor_id {
                    return Err(EngineError::BotPermissionDenied);
                }
                self.require_actor(&actor_id)?;
                self.require_conversation(&invocation.conversation_id)?;
                let mut registry = self.state.bots.clone();
                let execution = registry
                    .begin_execution(&invocation, created_at_ms)
                    .map_err(|error| EngineError::Bot(error.to_string()))?;
                let profile = registry.bots.get(&execution.bot_id).cloned();
                Ok(vec![Event::BotRegistryChanged {
                    registry,
                    profile,
                    execution: Some(execution),
                }])
            }
            Command::FinishBotExecution {
                actor_id,
                execution_id,
                success,
                finished_at_ms,
                error,
            } => {
                let mut registry = self.state.bots.clone();
                let execution =
                    registry
                        .executions
                        .get(&execution_id)
                        .cloned()
                        .ok_or_else(|| {
                            EngineError::Bot(format!("execution not found: {execution_id}"))
                        })?;
                if execution.bot_id != actor_id {
                    return Err(EngineError::BotPermissionDenied);
                }
                registry
                    .finish_execution(&execution_id, success, finished_at_ms, error)
                    .map_err(|error| EngineError::Bot(error.to_string()))?;
                let execution = registry.executions.get(&execution_id).cloned();
                let profile = execution
                    .as_ref()
                    .and_then(|execution| registry.bots.get(&execution.bot_id))
                    .cloned();
                Ok(vec![Event::BotRegistryChanged {
                    registry,
                    profile,
                    execution,
                }])
            }
            Command::InstallMiniApp { manifest } => Ok(vec![Event::MiniAppInstalled { manifest }]),
            Command::GrantMiniApp { grant } => {
                if !self.state.mini_apps.contains_key(&grant.mini_app_id) {
                    return Err(EngineError::MiniAppNotFound(grant.mini_app_id));
                }
                self.require_actor(&grant.actor_id)?;
                Ok(vec![Event::MiniAppGrantUpdated { grant }])
            }
            Command::OpenMiniApp { session } => {
                if !self.state.mini_apps.contains_key(&session.mini_app_id) {
                    return Err(EngineError::MiniAppNotFound(session.mini_app_id));
                }
                let grant = self
                    .state
                    .mini_app_grants
                    .get(&(session.mini_app_id.clone(), session.actor_id.clone()));
                if session.granted_permissions.iter().any(|permission| {
                    !grant.is_some_and(|grant| grant.permissions.contains(permission))
                }) {
                    return Err(EngineError::MiniAppPermissionDenied(
                        *session
                            .granted_permissions
                            .iter()
                            .find(|permission| {
                                !grant.is_some_and(|grant| grant.permissions.contains(permission))
                            })
                            .expect("permission exists"),
                    ));
                }
                Ok(vec![Event::MiniAppOpened { session }])
            }
            Command::MiniAppCall {
                session_id,
                request_id,
                request,
            } => {
                let session = self
                    .state
                    .mini_app_sessions
                    .get(&session_id)
                    .ok_or_else(|| EngineError::MiniAppSessionNotFound(session_id.clone()))?;
                if let Some(permission) = request.required_permission() {
                    if !session.granted_permissions.contains(&permission) {
                        return Err(EngineError::MiniAppPermissionDenied(permission));
                    }
                }
                let response = match request {
                    MiniAppRequest::Ready
                    | MiniAppRequest::Expand
                    | MiniAppRequest::Close
                    | MiniAppRequest::SetHeaderColor { .. }
                    | MiniAppRequest::SetBackgroundColor { .. } => MiniAppResponse::Ok,
                    _ => return Err(EngineError::MiniAppHostActionRequired),
                };
                Ok(vec![Event::MiniAppResponded {
                    session_id,
                    request_id,
                    response,
                }])
            }
        }
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::ActorUpserted { actor } => {
                self.state.actors.insert(actor.id.clone(), actor);
            }
            Event::PresenceUpdated { actor_id, presence } => {
                if let Some(actor) = self.state.actors.get_mut(&actor_id) {
                    actor.presence = presence;
                }
            }
            Event::ConversationUpserted { conversation } => {
                self.state
                    .conversations
                    .insert(conversation.id.clone(), conversation);
            }
            Event::ConversationInfoUpdated {
                conversation_id,
                title,
                description,
            } => {
                if let Some(conversation) = self.state.conversations.get_mut(&conversation_id) {
                    conversation.title = title;
                    conversation.description = description;
                }
            }
            Event::ConversationParticipantUpserted {
                conversation_id,
                participant,
            } => {
                if let Some(conversation) = self.state.conversations.get_mut(&conversation_id) {
                    if let Some(existing) = conversation
                        .participants
                        .iter_mut()
                        .find(|item| item.actor_id == participant.actor_id)
                    {
                        *existing = participant;
                    } else {
                        conversation.participants.push(participant);
                    }
                }
            }
            Event::ConversationParticipantRemoved {
                conversation_id,
                actor_id,
            } => {
                if let Some(conversation) = self.state.conversations.get_mut(&conversation_id) {
                    conversation
                        .participants
                        .retain(|participant| participant.actor_id != actor_id);
                }
                if let Some(cursors) = self.state.read_cursors.get_mut(&conversation_id) {
                    cursors.remove(&actor_id);
                }
                if let Some(drafts) = self.state.drafts.get_mut(&conversation_id) {
                    drafts.remove(&actor_id);
                }
                if let Some(marked) = self.state.marked_unread_by_actor.get_mut(&conversation_id) {
                    marked.remove(&actor_id);
                }
            }
            Event::ConversationArchived {
                conversation_id,
                archived,
            } => {
                if let Some(conversation) = self.state.conversations.get_mut(&conversation_id) {
                    conversation.archived = archived;
                }
            }
            Event::ConversationPinned {
                conversation_id,
                pinned,
            } => {
                if let Some(conversation) = self.state.conversations.get_mut(&conversation_id) {
                    conversation.pinned = pinned;
                }
            }
            Event::ConversationMarkedUnread {
                conversation_id,
                actor_id,
                marked_unread,
            } => {
                let actors = self
                    .state
                    .marked_unread_by_actor
                    .entry(conversation_id.clone())
                    .or_default();
                if marked_unread {
                    actors.insert(actor_id.clone());
                } else {
                    actors.remove(&actor_id);
                }
                if actors.is_empty() {
                    self.state.marked_unread_by_actor.remove(&conversation_id);
                }
                if let Some(conversation) = self.state.conversations.get_mut(&conversation_id) {
                    conversation.marked_unread = marked_unread;
                }
            }
            Event::DraftChanged { draft } => {
                if draft.text.trim().is_empty() && draft.reply_to_message_id.is_none() {
                    if let Some(by_actor) = self.state.drafts.get_mut(&draft.conversation_id) {
                        by_actor.remove(&draft.actor_id);
                        if by_actor.is_empty() {
                            self.state.drafts.remove(&draft.conversation_id);
                        }
                    }
                } else {
                    self.state
                        .drafts
                        .entry(draft.conversation_id.clone())
                        .or_default()
                        .insert(draft.actor_id.clone(), draft);
                }
            }
            Event::ConversationNotificationsUpdated {
                conversation_id,
                settings,
            } => {
                if let Some(conversation) = self.state.conversations.get_mut(&conversation_id) {
                    conversation.notification_settings = settings;
                }
            }
            Event::FolderUpserted { folder } => {
                self.state.folders.insert(folder.id.clone(), folder);
            }
            Event::FolderDeleted { folder_id } => {
                self.state.folders.remove(&folder_id);
                for conversation in self.state.conversations.values_mut() {
                    conversation.folder_ids.retain(|id| id != &folder_id);
                }
            }
            Event::MessageQueued { message } => {
                if let Some(conversation) =
                    self.state.conversations.get_mut(&message.conversation_id)
                {
                    conversation.last_message_id = Some(message.id.0.clone());
                    conversation.updated_at_ms = message.created_at_ms;
                }
                self.state
                    .messages
                    .entry(message.conversation_id.clone())
                    .or_default()
                    .insert(message.id.clone(), message);
            }
            Event::MessageAcknowledged {
                conversation_id,
                local_message_id,
                server_message_id,
                accepted_at_ms,
            } => {
                if let Some(messages) = self.state.messages.get_mut(&conversation_id) {
                    if let Some(mut message) = messages.remove(&local_message_id) {
                        message.id = server_message_id.clone();
                        message.delivery_state = DeliveryState::Sent;
                        message.created_at_ms = accepted_at_ms;
                        messages.insert(server_message_id.clone(), message);
                    }
                }
                if let Some(conversation) = self.state.conversations.get_mut(&conversation_id) {
                    if conversation.last_message_id.as_deref() == Some(local_message_id.0.as_str())
                    {
                        conversation.last_message_id = Some(server_message_id.0);
                    }
                }
            }
            Event::DeliveryStateUpdated {
                conversation_id,
                message_id,
                state,
            } => {
                if let Some(message) = self
                    .state
                    .messages
                    .get_mut(&conversation_id)
                    .and_then(|messages| messages.get_mut(&message_id))
                {
                    message.delivery_state = state;
                }
            }
            Event::MessageEdited {
                conversation_id,
                message_id,
                content,
                edited_at_ms,
            } => {
                if let Some(message) = self
                    .state
                    .messages
                    .get_mut(&conversation_id)
                    .and_then(|messages| messages.get_mut(&message_id))
                {
                    message.content = content;
                    message.edited_at_ms = Some(edited_at_ms);
                }
            }
            Event::MessagesDeleted {
                conversation_id,
                message_ids,
            } => {
                if let Some(messages) = self.state.messages.get_mut(&conversation_id) {
                    for id in message_ids {
                        if let Some(message) = messages.get_mut(&id) {
                            message.deleted = true;
                        }
                    }
                }
            }
            Event::ConversationRead {
                conversation_id,
                actor_id,
                message_id,
            } => {
                self.state
                    .read_cursors
                    .entry(conversation_id.clone())
                    .or_default()
                    .insert(actor_id.clone(), message_id.clone());
                if let Some(actors) = self.state.marked_unread_by_actor.get_mut(&conversation_id) {
                    actors.remove(&actor_id);
                    if actors.is_empty() {
                        self.state.marked_unread_by_actor.remove(&conversation_id);
                    }
                }
                // Keep legacy snapshot fields coherent for older readers. Actor-specific
                // clients receive the authoritative read state from the projected sync view.
                if let Some(conversation) = self.state.conversations.get_mut(&conversation_id) {
                    conversation.last_read_message_id = Some(message_id.0);
                    conversation.unread_count = 0;
                    conversation.marked_unread = false;
                }
            }
            Event::ReactionUpdated {
                conversation_id,
                message_id,
                reaction,
            } => {
                if let Some(message) = self
                    .state
                    .messages
                    .get_mut(&conversation_id)
                    .and_then(|messages| messages.get_mut(&message_id))
                {
                    message
                        .reactions
                        .retain(|item| item.reaction != reaction.reaction);
                    if reaction.count > 0 {
                        message.reactions.push(reaction);
                    }
                }
            }
            Event::MessagePinned {
                conversation_id,
                message_id,
                pinned,
            } => {
                if let Some(message) = self
                    .state
                    .messages
                    .get_mut(&conversation_id)
                    .and_then(|messages| messages.get_mut(&message_id))
                {
                    message.pinned = pinned;
                }
                if let Some(conversation) = self.state.conversations.get_mut(&conversation_id) {
                    conversation
                        .pinned_message_ids
                        .retain(|id| id != &message_id.0);
                    if pinned {
                        conversation.pinned_message_ids.push(message_id.0);
                    }
                }
            }
            Event::PollVoteChanged {
                conversation_id,
                message_id,
                actor_id,
                option_ids,
            } => {
                let selected = option_ids.into_iter().collect::<BTreeSet<_>>();
                let by_message = self
                    .state
                    .poll_votes
                    .entry(conversation_id.clone())
                    .or_default();
                let by_actor = by_message.entry(message_id.clone()).or_default();
                if selected.is_empty() {
                    by_actor.remove(&actor_id);
                } else {
                    by_actor.insert(actor_id, selected);
                }
                if let Some(message) = self
                    .state
                    .messages
                    .get_mut(&conversation_id)
                    .and_then(|messages| messages.get_mut(&message_id))
                {
                    if let MessageContent::Poll { options, .. } = &mut message.content {
                        for option in options {
                            option.voter_count = u32::try_from(
                                by_actor
                                    .values()
                                    .filter(|votes| votes.contains(&option.id))
                                    .count(),
                            )
                            .unwrap_or(u32::MAX);
                            option.chosen = false;
                        }
                    }
                }
            }
            Event::InvoiceCreated { invoice } => {
                self.state.invoices.insert(invoice.id.clone(), invoice);
            }
            Event::OrderUpserted { order } => {
                self.state.orders.insert(order.id.clone(), order);
            }
            Event::WalletChanged { wallet, .. } => {
                self.state.wallet = wallet;
            }
            Event::StoryChanged { story } => {
                self.state.stories.insert(story.id.clone(), story);
            }
            Event::StoryDeleted { story_id } => {
                self.state.stories.remove(&story_id);
            }
            Event::CommunityChanged { community } => {
                self.state
                    .communities
                    .insert(community.conversation_id.clone(), community);
            }
            Event::BotRegistryChanged { registry, .. } => {
                self.state.bots = registry;
            }
            Event::MiniAppInstalled { manifest } => {
                self.state.mini_apps.insert(manifest.id.clone(), manifest);
            }
            Event::MiniAppGrantUpdated { grant } => {
                self.state
                    .mini_app_grants
                    .insert((grant.mini_app_id.clone(), grant.actor_id.clone()), grant);
            }
            Event::MiniAppOpened { session } => {
                self.state
                    .mini_app_sessions
                    .insert(session.id.clone(), session);
            }
            Event::MiniAppResponded { .. } => {}
        }
    }

    pub fn complete_paid_order(
        &mut self,
        order_id: &str,
        updated_at_ms: i64,
    ) -> Result<Vec<Event>, EngineError> {
        let mut order = self
            .state
            .orders
            .get(order_id)
            .cloned()
            .ok_or_else(|| EngineError::OrderNotFound(order_id.to_string()))?;
        order.status = PaymentStatus::Paid;
        order.updated_at_ms = updated_at_ms;
        self.execute(Command::UpsertOrder { order })
    }

    fn require_actor(&self, id: &ActorId) -> Result<&Actor, EngineError> {
        self.state
            .actors
            .get(id)
            .ok_or_else(|| EngineError::ActorNotFound(id.clone()))
    }
    fn require_conversation(&self, id: &ConversationId) -> Result<&Conversation, EngineError> {
        self.state
            .conversations
            .get(id)
            .ok_or_else(|| EngineError::ConversationNotFound(id.clone()))
    }
    fn require_message(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<&Message, EngineError> {
        self.state
            .messages
            .get(conversation_id)
            .and_then(|messages| messages.get(message_id))
            .ok_or_else(|| EngineError::MessageNotFound {
                conversation_id: conversation_id.clone(),
                message_id: message_id.clone(),
            })
    }
}
