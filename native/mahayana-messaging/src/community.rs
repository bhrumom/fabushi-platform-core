use crate::actor::ActorId;
use crate::conversation::ConversationId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MemberStatus {
    Owner,
    Administrator,
    Member,
    Restricted,
    Left,
    Banned,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminRights {
    pub change_info: bool,
    pub post_messages: bool,
    pub edit_messages: bool,
    pub delete_messages: bool,
    pub ban_members: bool,
    pub invite_members: bool,
    pub pin_messages: bool,
    pub manage_topics: bool,
    pub manage_calls: bool,
    pub add_admins: bool,
    pub remain_anonymous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberRestrictions {
    pub send_messages: bool,
    pub send_media: bool,
    pub send_polls: bool,
    pub embed_links: bool,
    pub add_members: bool,
    pub pin_messages: bool,
    pub change_info: bool,
    pub until_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityMember {
    pub actor_id: ActorId,
    pub status: MemberStatus,
    pub admin_title: Option<String>,
    pub admin_rights: AdminRights,
    pub restrictions: MemberRestrictions,
    pub joined_at_ms: i64,
    pub invited_by: Option<ActorId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteLink {
    pub id: String,
    pub conversation_id: ConversationId,
    pub creator_id: ActorId,
    pub token: String,
    pub name: Option<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub member_limit: Option<u32>,
    pub join_request: bool,
    pub revoked: bool,
    pub joined_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequest {
    pub conversation_id: ConversationId,
    pub actor_id: ActorId,
    pub invite_link_id: Option<String>,
    pub bio: Option<String>,
    pub requested_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForumTopicState {
    pub id: String,
    pub conversation_id: ConversationId,
    pub title: String,
    pub icon: Option<String>,
    pub creator_id: ActorId,
    pub created_at_ms: i64,
    pub pinned: bool,
    pub closed: bool,
    pub hidden: bool,
    pub unread_count: u32,
    pub last_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSubscription {
    pub actor_id: ActorId,
    pub subscribed_at_ms: i64,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunityAuditAction {
    CommunityUpdated,
    SubscriptionAdded,
    SubscriptionRemoved,
    MemberChanged,
    InviteCreated,
    InviteRevoked,
    JoinApproved,
    JoinRejected,
    TopicUpserted,
    TopicDeleted,
    SlowModeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityAuditEntry {
    pub id: String,
    pub actor_id: ActorId,
    pub action: CommunityAuditAction,
    pub target_actor_id: Option<ActorId>,
    pub target_id: Option<String>,
    pub reason: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityState {
    pub conversation_id: ConversationId,
    pub public_username: Option<String>,
    pub linked_discussion_id: Option<ConversationId>,
    pub signatures_enabled: bool,
    pub join_to_send: bool,
    pub join_request_required: bool,
    pub slow_mode_seconds: Option<u32>,
    pub members: BTreeMap<ActorId, CommunityMember>,
    pub invite_links: BTreeMap<String, InviteLink>,
    pub pending_join_requests: BTreeMap<ActorId, JoinRequest>,
    pub topics: BTreeMap<String, ForumTopicState>,
    pub banned_words: BTreeSet<String>,
    #[serde(default)]
    pub subscribers: BTreeMap<ActorId, ChannelSubscription>,
    #[serde(default)]
    pub admin_log: Vec<CommunityAuditEntry>,
}

impl CommunityState {
    pub fn new(conversation_id: ConversationId) -> Self {
        Self {
            conversation_id,
            public_username: None,
            linked_discussion_id: None,
            signatures_enabled: false,
            join_to_send: false,
            join_request_required: false,
            slow_mode_seconds: None,
            members: BTreeMap::new(),
            invite_links: BTreeMap::new(),
            pending_join_requests: BTreeMap::new(),
            topics: BTreeMap::new(),
            banned_words: BTreeSet::new(),
            subscribers: BTreeMap::new(),
            admin_log: Vec::new(),
        }
    }

    pub fn is_subscriber(&self, actor_id: &ActorId) -> bool {
        self.subscribers.contains_key(actor_id)
    }

    pub fn subscribe_channel(
        &mut self,
        actor_id: ActorId,
        subscribed_at_ms: i64,
    ) -> Result<(), CommunityError> {
        if self.members.get(&actor_id).is_some_and(|member| {
            matches!(member.status, MemberStatus::Banned | MemberStatus::Left)
        }) {
            return Err(CommunityError::PermissionDenied);
        }
        self.subscribers
            .entry(actor_id.clone())
            .or_insert(ChannelSubscription {
                actor_id,
                subscribed_at_ms,
                muted: false,
            });
        Ok(())
    }

    pub fn unsubscribe_channel(&mut self, actor_id: &ActorId) -> Result<(), CommunityError> {
        if is_owner(self, actor_id) {
            return Err(CommunityError::OwnerCannotUnsubscribe);
        }
        self.subscribers.remove(actor_id);
        Ok(())
    }

    pub fn append_audit(&mut self, entry: CommunityAuditEntry) {
        self.admin_log.push(entry);
        const MAX_AUDIT_ENTRIES: usize = 1_000;
        if self.admin_log.len() > MAX_AUDIT_ENTRIES {
            let excess = self.admin_log.len() - MAX_AUDIT_ENTRIES;
            self.admin_log.drain(0..excess);
        }
    }

    pub fn member_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> (Vec<CommunityMember>, Option<String>) {
        let mut members = self.members.values().cloned().collect::<Vec<_>>();
        for subscription in self.subscribers.values() {
            if !self.members.contains_key(&subscription.actor_id) {
                members.push(CommunityMember {
                    actor_id: subscription.actor_id.clone(),
                    status: MemberStatus::Member,
                    admin_title: None,
                    admin_rights: AdminRights::default(),
                    restrictions: MemberRestrictions::default(),
                    joined_at_ms: subscription.subscribed_at_ms,
                    invited_by: None,
                });
            }
        }
        members.sort_by(|left, right| left.actor_id.cmp(&right.actor_id));
        page_by_actor_id(members, cursor, limit)
    }

    pub fn audit_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> (Vec<CommunityAuditEntry>, Option<String>) {
        let mut entries = self.admin_log.clone();
        entries.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        let start = cursor
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            .min(entries.len());
        let page_limit = limit.clamp(1, 100);
        let end = start.saturating_add(page_limit).min(entries.len());
        let next = (end < entries.len()).then(|| end.to_string());
        (
            entries.into_iter().skip(start).take(end - start).collect(),
            next,
        )
    }

    pub fn can_moderate(&self, actor_id: &ActorId) -> bool {
        self.members.get(actor_id).is_some_and(|member| {
            matches!(member.status, MemberStatus::Owner)
                || (matches!(member.status, MemberStatus::Administrator)
                    && member.admin_rights.delete_messages)
        })
    }

    pub fn can_manage_invites(&self, actor_id: &ActorId) -> bool {
        self.members.get(actor_id).is_some_and(|member| {
            matches!(member.status, MemberStatus::Owner)
                || (matches!(member.status, MemberStatus::Administrator)
                    && member.admin_rights.invite_members)
        })
    }

    pub fn upsert_member(&mut self, member: CommunityMember) {
        self.pending_join_requests.remove(&member.actor_id);
        self.members.insert(member.actor_id.clone(), member);
    }

    pub fn request_join(&mut self, request: JoinRequest) {
        self.pending_join_requests
            .insert(request.actor_id.clone(), request);
    }

    pub fn approve_join(
        &mut self,
        actor_id: &ActorId,
        approved_by: &ActorId,
        now_ms: i64,
    ) -> Result<(), CommunityError> {
        if !self.can_manage_invites(approved_by) {
            return Err(CommunityError::PermissionDenied);
        }
        let request = self
            .pending_join_requests
            .remove(actor_id)
            .ok_or_else(|| CommunityError::JoinRequestNotFound(actor_id.clone()))?;
        self.upsert_member(CommunityMember {
            actor_id: request.actor_id,
            status: MemberStatus::Member,
            admin_title: None,
            admin_rights: AdminRights::default(),
            restrictions: MemberRestrictions::default(),
            joined_at_ms: now_ms,
            invited_by: Some(approved_by.clone()),
        });
        Ok(())
    }

    pub fn revoke_invite(&mut self, invite_id: &str) -> Result<(), CommunityError> {
        let invite = self
            .invite_links
            .get_mut(invite_id)
            .ok_or_else(|| CommunityError::InviteNotFound(invite_id.to_string()))?;
        invite.revoked = true;
        Ok(())
    }
}

fn is_owner(community: &CommunityState, actor_id: &ActorId) -> bool {
    community
        .members
        .get(actor_id)
        .is_some_and(|member| matches!(member.status, MemberStatus::Owner))
}

fn page_by_actor_id(
    members: Vec<CommunityMember>,
    cursor: Option<&str>,
    limit: usize,
) -> (Vec<CommunityMember>, Option<String>) {
    let start = cursor
        .and_then(|value| {
            members
                .iter()
                .position(|member| member.actor_id.0.as_str() > value)
        })
        .unwrap_or_else(|| if cursor.is_some() { members.len() } else { 0 });
    let page_limit = limit.clamp(1, 100);
    let end = start.saturating_add(page_limit).min(members.len());
    let next = (end < members.len()).then(|| members[end - 1].actor_id.0.clone());
    (
        members.into_iter().skip(start).take(end - start).collect(),
        next,
    )
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CommunityError {
    #[error("community permission denied")]
    PermissionDenied,
    #[error("join request for actor {0:?} was not found")]
    JoinRequestNotFound(ActorId),
    #[error("invite link {0} was not found")]
    InviteNotFound(String),
    #[error("community owner cannot unsubscribe from their own channel")]
    OwnerCannotUnsubscribe,
}
