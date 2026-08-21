use crate::actor::{Actor, ActorId};
use crate::conversation::{Conversation, ConversationId};
use crate::message::{Message, MessageContent, MessageId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchScope {
    Global,
    Conversation,
    Contacts,
    Bots,
    Groups,
    Channels,
    Media,
    Files,
    Links,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub text: String,
    pub scope: SearchScope,
    pub conversation_id: Option<ConversationId>,
    pub sender_id: Option<ActorId>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchResultKind {
    Actor,
    Conversation,
    Message,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub kind: SearchResultKind,
    pub id: String,
    pub conversation_id: Option<ConversationId>,
    pub title: String,
    pub snippet: String,
    pub timestamp_ms: Option<i64>,
    pub score: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndex {
    actor_terms: BTreeMap<ActorId, BTreeSet<String>>,
    conversation_terms: BTreeMap<ConversationId, BTreeSet<String>>,
    message_terms: BTreeMap<(ConversationId, MessageId), BTreeSet<String>>,
    actors: BTreeMap<ActorId, Actor>,
    conversations: BTreeMap<ConversationId, Conversation>,
    messages: BTreeMap<(ConversationId, MessageId), Message>,
}

impl SearchIndex {
    pub fn index_actor(&mut self, actor: Actor) {
        let mut text = actor.display_name.clone();
        if let Some(username) = &actor.username {
            text.push(' ');
            text.push_str(username);
        }
        if let Some(bio) = &actor.bio {
            text.push(' ');
            text.push_str(bio);
        }
        self.actor_terms.insert(actor.id.clone(), tokenize(&text));
        self.actors.insert(actor.id.clone(), actor);
    }

    pub fn index_conversation(&mut self, conversation: Conversation) {
        let text = format!(
            "{} {}",
            conversation.title,
            conversation.description.as_deref().unwrap_or_default()
        );
        self.conversation_terms
            .insert(conversation.id.clone(), tokenize(&text));
        self.conversations
            .insert(conversation.id.clone(), conversation);
    }

    pub fn index_message(&mut self, message: Message) {
        let terms = tokenize(&message_search_text(&message));
        self.message_terms
            .insert((message.conversation_id.clone(), message.id.clone()), terms);
        self.messages.insert(
            (message.conversation_id.clone(), message.id.clone()),
            message,
        );
    }

    pub fn remove_message(&mut self, conversation_id: &ConversationId, message_id: &MessageId) {
        let key = (conversation_id.clone(), message_id.clone());
        self.message_terms.remove(&key);
        self.messages.remove(&key);
    }

    pub fn search(&self, query: &SearchQuery) -> Vec<SearchResult> {
        let terms = tokenize(&query.text);
        if terms.is_empty() {
            return Vec::new();
        }
        let mut results = Vec::new();
        if matches!(
            query.scope,
            SearchScope::Global | SearchScope::Contacts | SearchScope::Bots
        ) {
            for (id, indexed) in &self.actor_terms {
                let Some(actor) = self.actors.get(id) else {
                    continue;
                };
                let allowed = match query.scope {
                    SearchScope::Contacts => matches!(actor.kind, crate::actor::ActorKind::Human),
                    SearchScope::Bots => matches!(
                        actor.kind,
                        crate::actor::ActorKind::Bot | crate::actor::ActorKind::Assistant
                    ),
                    _ => true,
                };
                if allowed {
                    let score = score_terms(&terms, indexed);
                    if score > 0 {
                        results.push(SearchResult {
                            kind: SearchResultKind::Actor,
                            id: id.0.clone(),
                            conversation_id: None,
                            title: actor.display_name.clone(),
                            snippet: actor.bio.clone().unwrap_or_default(),
                            timestamp_ms: actor.presence.last_seen_at_ms,
                            score,
                        });
                    }
                }
            }
        }
        if matches!(
            query.scope,
            SearchScope::Global | SearchScope::Groups | SearchScope::Channels
        ) {
            for (id, indexed) in &self.conversation_terms {
                let Some(conversation) = self.conversations.get(id) else {
                    continue;
                };
                let allowed = match query.scope {
                    SearchScope::Groups => matches!(
                        conversation.kind,
                        crate::conversation::ConversationKind::Group
                    ),
                    SearchScope::Channels => matches!(
                        conversation.kind,
                        crate::conversation::ConversationKind::Channel
                    ),
                    _ => true,
                };
                if allowed {
                    let score = score_terms(&terms, indexed);
                    if score > 0 {
                        results.push(SearchResult {
                            kind: SearchResultKind::Conversation,
                            id: id.0.clone(),
                            conversation_id: Some(id.clone()),
                            title: conversation.title.clone(),
                            snippet: conversation.description.clone().unwrap_or_default(),
                            timestamp_ms: Some(conversation.updated_at_ms),
                            score,
                        });
                    }
                }
            }
        }
        if matches!(
            query.scope,
            SearchScope::Global
                | SearchScope::Conversation
                | SearchScope::Media
                | SearchScope::Files
                | SearchScope::Links
        ) {
            for ((conversation_id, message_id), indexed) in &self.message_terms {
                let Some(message) = self
                    .messages
                    .get(&(conversation_id.clone(), message_id.clone()))
                else {
                    continue;
                };
                if query
                    .conversation_id
                    .as_ref()
                    .is_some_and(|id| id != conversation_id)
                    || query
                        .sender_id
                        .as_ref()
                        .is_some_and(|id| id != &message.sender_id)
                    || query
                        .from_ms
                        .is_some_and(|from| message.created_at_ms < from)
                    || query.to_ms.is_some_and(|to| message.created_at_ms > to)
                    || !content_matches_scope(&message.content, query.scope)
                {
                    continue;
                }
                let score = score_terms(&terms, indexed);
                if score > 0 {
                    results.push(SearchResult {
                        kind: SearchResultKind::Message,
                        id: message_id.0.clone(),
                        conversation_id: Some(conversation_id.clone()),
                        title: self
                            .conversations
                            .get(conversation_id)
                            .map(|value| value.title.clone())
                            .unwrap_or_else(|| conversation_id.0.clone()),
                        snippet: message_search_text(message),
                        timestamp_ms: Some(message.created_at_ms),
                        score,
                    });
                }
            }
        }
        results.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.timestamp_ms.cmp(&left.timestamp_ms))
        });
        results.truncate(usize::try_from(query.limit.max(1)).unwrap_or(usize::MAX));
        results
    }
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|character: char| {
            !character.is_alphanumeric() && character != '_' && character != '@' && character != '#'
        })
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn score_terms(query: &BTreeSet<String>, indexed: &BTreeSet<String>) -> u32 {
    query
        .iter()
        .map(|term| {
            if indexed.contains(term) {
                10
            } else if indexed.iter().any(|candidate| candidate.contains(term)) {
                4
            } else {
                0
            }
        })
        .sum()
}

fn message_search_text(message: &Message) -> String {
    match &message.content {
        MessageContent::Text { text } => text.text.clone(),
        MessageContent::Photo { caption, .. }
        | MessageContent::Video { caption, .. }
        | MessageContent::Animation { caption, .. }
        | MessageContent::Audio { caption, .. }
        | MessageContent::Voice { caption, .. }
        | MessageContent::Document { caption, .. } => caption.text.clone(),
        MessageContent::Contact {
            display_name,
            phone_number,
            ..
        } => format!(
            "{} {}",
            display_name,
            phone_number.as_deref().unwrap_or_default()
        ),
        MessageContent::Poll { question, .. } => question.text.clone(),
        MessageContent::Venue { title, address, .. } => format!("{title} {address}"),
        MessageContent::Service { action, text } => {
            format!("{} {}", action, text.as_deref().unwrap_or_default())
        }
        MessageContent::Invoice { invoice_id } => invoice_id.clone(),
        MessageContent::MiniApp {
            mini_app_id, title, ..
        } => format!("{title} {mini_app_id}"),
        _ => String::new(),
    }
}

fn content_matches_scope(content: &MessageContent, scope: SearchScope) -> bool {
    match scope {
        SearchScope::Media => matches!(
            content,
            MessageContent::Photo { .. }
                | MessageContent::Video { .. }
                | MessageContent::Animation { .. }
                | MessageContent::Audio { .. }
                | MessageContent::Voice { .. }
                | MessageContent::VideoNote { .. }
                | MessageContent::Sticker { .. }
        ),
        SearchScope::Files => matches!(content, MessageContent::Document { .. }),
        SearchScope::Links => match content {
            MessageContent::Text { text } => text.entities.iter().any(|entity| {
                matches!(
                    &entity.kind,
                    crate::message::TextEntityKind::Url
                        | crate::message::TextEntityKind::TextUrl(_)
                )
            }),
            _ => false,
        },
        _ => true,
    }
}
