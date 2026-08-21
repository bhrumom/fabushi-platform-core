use crate::actor::ActorId;
use crate::message::{FormattedText, MediaRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StoryId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoryPrivacyKind {
    Everyone,
    Contacts,
    CloseFriends,
    Selected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryPrivacy {
    pub kind: StoryPrivacyKind,
    pub included_actor_ids: BTreeSet<ActorId>,
    pub excluded_actor_ids: BTreeSet<ActorId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryView {
    pub actor_id: ActorId,
    pub viewed_at_ms: i64,
    pub reaction: Option<String>,
    pub forwarded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Story {
    pub id: StoryId,
    pub owner_id: ActorId,
    pub media: MediaRef,
    pub caption: FormattedText,
    pub privacy: StoryPrivacy,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub edited_at_ms: Option<i64>,
    pub pinned_to_profile: bool,
    pub protected_content: bool,
    pub allow_replies: bool,
    pub views: BTreeMap<ActorId, StoryView>,
}

impl Story {
    pub fn is_visible_to(
        &self,
        actor_id: &ActorId,
        is_contact: bool,
        is_close_friend: bool,
    ) -> bool {
        if &self.owner_id == actor_id {
            return true;
        }
        if self.privacy.excluded_actor_ids.contains(actor_id) {
            return false;
        }
        if self.privacy.included_actor_ids.contains(actor_id) {
            return true;
        }
        match self.privacy.kind {
            StoryPrivacyKind::Everyone => true,
            StoryPrivacyKind::Contacts => is_contact,
            StoryPrivacyKind::CloseFriends => is_close_friend,
            StoryPrivacyKind::Selected => false,
        }
    }

    pub fn record_view(&mut self, actor_id: ActorId, viewed_at_ms: i64) -> Result<(), StoryError> {
        if viewed_at_ms > self.expires_at_ms && !self.pinned_to_profile {
            return Err(StoryError::Expired(self.id.clone()));
        }
        self.views
            .entry(actor_id.clone())
            .and_modify(|view| view.viewed_at_ms = view.viewed_at_ms.min(viewed_at_ms))
            .or_insert(StoryView {
                actor_id,
                viewed_at_ms,
                reaction: None,
                forwarded: false,
            });
        Ok(())
    }

    pub fn react(
        &mut self,
        actor_id: &ActorId,
        reaction: Option<String>,
    ) -> Result<(), StoryError> {
        let view = self
            .views
            .get_mut(actor_id)
            .ok_or_else(|| StoryError::ViewerNotFound(actor_id.clone()))?;
        view.reaction = reaction;
        Ok(())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StoryError {
    #[error("story {0:?} has expired")]
    Expired(StoryId),
    #[error("story viewer {0:?} was not found")]
    ViewerNotFound(ActorId),
}
