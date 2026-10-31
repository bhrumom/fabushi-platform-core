use crate::actor::ActorId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrivacyAudience {
    Everyone,
    Contacts,
    Nobody,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyRule {
    pub audience: PrivacyAudience,
    pub always_allow: BTreeSet<ActorId>,
    pub never_allow: BTreeSet<ActorId>,
}

impl Default for PrivacyRule {
    fn default() -> Self {
        Self {
            audience: PrivacyAudience::Contacts,
            always_allow: BTreeSet::new(),
            never_allow: BTreeSet::new(),
        }
    }
}

impl PrivacyRule {
    pub fn allows(&self, actor_id: &ActorId, is_contact: bool) -> bool {
        if self.never_allow.contains(actor_id) {
            return false;
        }
        if self.always_allow.contains(actor_id) {
            return true;
        }
        match self.audience {
            PrivacyAudience::Everyone => true,
            PrivacyAudience::Contacts => is_contact,
            PrivacyAudience::Nobody | PrivacyAudience::Custom => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrivacyKey {
    LastSeen,
    ProfilePhoto,
    Bio,
    PhoneNumber,
    ForwardedMessages,
    Calls,
    GroupInvites,
    VoiceMessages,
    Stories,
    ReadReceipts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockEntry {
    pub actor_id: ActorId,
    pub blocked_at_ms: i64,
    pub reason: Option<String>,
    pub report_spam: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySettings {
    pub rules: BTreeMap<PrivacyKey, PrivacyRule>,
    pub blocked: BTreeMap<ActorId, BlockEntry>,
    pub auto_delete_account_after_days: Option<u32>,
    pub hide_sensitive_content: bool,
    pub require_mutual_contact_for_messages: bool,
}

impl PrivacySettings {
    pub fn rule(&self, key: PrivacyKey) -> PrivacyRule {
        self.rules.get(&key).cloned().unwrap_or_default()
    }

    pub fn block(&mut self, entry: BlockEntry) {
        self.blocked.insert(entry.actor_id.clone(), entry);
    }

    pub fn unblock(&mut self, actor_id: &ActorId) -> Option<BlockEntry> {
        self.blocked.remove(actor_id)
    }

    pub fn is_blocked(&self, actor_id: &ActorId) -> bool {
        self.blocked.contains_key(actor_id)
    }

    pub fn can_receive_message_from(
        &self,
        actor_id: &ActorId,
        is_contact: bool,
        is_mutual_contact: bool,
    ) -> bool {
        if self.is_blocked(actor_id) {
            return false;
        }
        if self.require_mutual_contact_for_messages && !is_mutual_contact {
            return false;
        }
        is_contact || !self.require_mutual_contact_for_messages
    }
}
