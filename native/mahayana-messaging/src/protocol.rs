use crate::actor::{Actor, ActorId, Presence};
use crate::blob_store::{BlobId, BlobMetadata, BlobUploadStatus};
use crate::bot::{BotExecution, BotInvocation, BotProfile};
use crate::community::{CommunityMember, CommunityState, ForumTopicState, InviteLink, JoinRequest};
use crate::conversation::{Conversation, ConversationFolder, ConversationId, NotificationSettings};
use crate::message::{ClientMessageId, Message, MessageContent, MessageId, ReactionSummary};
use crate::miniapp::{
    MiniAppGrant, MiniAppManifest, MiniAppRequest, MiniAppResponse, MiniAppSession,
};
use crate::payment::{CustomerInfo, Invoice, PaymentOrder};
use crate::search::{SearchQuery, SearchResult};
use crate::story::{Story, StoryId};
use crate::wallet::{LedgerEntry, WalletAccount};
use serde::{Deserialize, Serialize};

pub const FABUSHI_MESSAGING_PROTOCOL_VERSION: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestContext {
    pub request_id: String,
    pub device_id: String,
    pub actor_id: ActorId,
    pub session_id: String,
    pub sent_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientEnvelope {
    pub protocol_version: u16,
    pub context: RequestContext,
    pub command: ClientCommand,
}

impl ClientEnvelope {
    pub fn new(context: RequestContext, command: ClientCommand) -> Self {
        Self {
            protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
            context,
            command,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum ClientCommand {
    Sync {
        cursor: Option<String>,
        limit: u32,
    },
    Search {
        query: SearchQuery,
    },
    UpsertProfile {
        actor: Actor,
    },
    SetPresence {
        presence: Presence,
    },
    CreateConversation {
        conversation: Conversation,
    },
    UpdateConversation {
        conversation: Conversation,
    },
    ArchiveConversation {
        conversation_id: ConversationId,
        archived: bool,
    },
    PinConversation {
        conversation_id: ConversationId,
        pinned: bool,
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
    SendMessage {
        conversation_id: ConversationId,
        client_message_id: ClientMessageId,
        content: MessageContent,
        reply_to_message_id: Option<MessageId>,
        thread_root_message_id: Option<MessageId>,
        scheduled_at_ms: Option<i64>,
        silent: bool,
        protected_content: bool,
    },
    ForwardMessage {
        source_conversation_id: ConversationId,
        message_id: MessageId,
        destination_conversation_id: ConversationId,
        client_message_id: ClientMessageId,
    },
    BeginBlobUpload {
        metadata: BlobMetadata,
    },
    AppendBlobChunk {
        blob_id: BlobId,
        offset: u64,
        data_base64: String,
    },
    FinishBlobUpload {
        blob_id: BlobId,
    },
    DeleteBlob {
        blob_id: BlobId,
    },
    EditMessage {
        conversation_id: ConversationId,
        message_id: MessageId,
        content: MessageContent,
    },
    DeleteMessages {
        conversation_id: ConversationId,
        message_ids: Vec<MessageId>,
        for_everyone: bool,
    },
    MarkRead {
        conversation_id: ConversationId,
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
    StartTyping {
        conversation_id: ConversationId,
        action: String,
    },
    StopTyping {
        conversation_id: ConversationId,
    },
    CreateInvoice {
        invoice: Invoice,
    },
    CheckoutInvoice {
        invoice_id: String,
        order_id: String,
        customer: Option<CustomerInfo>,
    },
    RefundOrder {
        order_id: String,
        request_id: String,
    },
    WalletStatus,
    PublishStory {
        story: Story,
    },
    DeleteStory {
        story_id: StoryId,
    },
    ViewStory {
        story_id: StoryId,
    },
    ReactStory {
        story_id: StoryId,
        reaction: Option<String>,
    },
    UpdateCommunity {
        community: CommunityState,
    },
    SetCommunityMember {
        conversation_id: ConversationId,
        member: CommunityMember,
    },
    CreateInviteLink {
        invite: InviteLink,
    },
    RevokeInviteLink {
        conversation_id: ConversationId,
        invite_id: String,
    },
    RequestCommunityJoin {
        request: JoinRequest,
    },
    RespondCommunityJoin {
        conversation_id: ConversationId,
        requester_id: ActorId,
        approved: bool,
    },
    UpsertForumTopic {
        topic: ForumTopicState,
    },
    DeleteForumTopic {
        conversation_id: ConversationId,
        topic_id: String,
    },
    RegisterBot {
        profile: BotProfile,
    },
    InvokeBot {
        invocation: BotInvocation,
    },
    FinishBotExecution {
        execution_id: String,
        success: bool,
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
#[serde(rename_all = "camelCase")]
pub struct ServerEnvelope {
    pub protocol_version: u16,
    pub cursor: Option<String>,
    pub server_time_ms: i64,
    pub event: ServerEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum ServerEvent {
    SyncBatch {
        actors: Vec<Actor>,
        conversations: Vec<Conversation>,
        messages: Vec<Message>,
        folders: Vec<ConversationFolder>,
        invoices: Vec<Invoice>,
        orders: Vec<PaymentOrder>,
        stories: Vec<Story>,
        communities: Vec<CommunityState>,
        bots: Vec<BotProfile>,
        bot_executions: Vec<BotExecution>,
        mini_apps: Vec<MiniAppManifest>,
        next_cursor: Option<String>,
    },
    SearchResults {
        query: SearchQuery,
        results: Vec<SearchResult>,
    },
    ActorChanged {
        actor: Actor,
    },
    PresenceChanged {
        actor_id: ActorId,
        presence: Presence,
    },
    ConversationChanged {
        conversation: Conversation,
    },
    FolderChanged {
        folder: ConversationFolder,
    },
    FolderDeleted {
        folder_id: String,
    },
    MessageAdded {
        message: Message,
    },
    MessageChanged {
        message: Message,
    },
    BlobUploadChanged {
        status: BlobUploadStatus,
    },
    BlobReady {
        metadata: BlobMetadata,
    },
    BlobDeleted {
        blob_id: BlobId,
    },
    MessagesDeleted {
        conversation_id: ConversationId,
        message_ids: Vec<MessageId>,
    },
    ReadChanged {
        conversation_id: ConversationId,
        actor_id: ActorId,
        message_id: MessageId,
    },
    TypingChanged {
        conversation_id: ConversationId,
        actor_id: ActorId,
        action: Option<String>,
        expires_at_ms: Option<i64>,
    },
    InvoiceChanged {
        invoice: Invoice,
    },
    OrderChanged {
        order: PaymentOrder,
    },
    WalletStatus {
        account: Option<WalletAccount>,
        recent_entries: Vec<LedgerEntry>,
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
    BotChanged {
        profile: Option<BotProfile>,
        execution: Option<BotExecution>,
    },
    BotInvocationRequested {
        invocation: BotInvocation,
    },
    MiniAppChanged {
        manifest: MiniAppManifest,
    },
    MiniAppOpened {
        session: MiniAppSession,
    },
    MiniAppResult {
        session_id: String,
        request_id: String,
        response: MiniAppResponse,
    },
    Error {
        request_id: Option<String>,
        code: String,
        message: String,
        retryable: bool,
    },
}
