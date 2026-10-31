use crate::actor::ActorId;
use crate::realtime::{CallId, CallSignal};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, Receiver, Sender};
use thiserror::Error;

#[derive(Debug)]
pub struct SignalSubscription {
    pub actor_id: ActorId,
    pub receiver: Receiver<CallSignal>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalingSnapshot {
    pub connected_actor_ids: BTreeSet<ActorId>,
    pub rooms: BTreeMap<CallId, BTreeSet<ActorId>>,
}

#[derive(Debug, Default)]
pub struct SignalingHub {
    clients: BTreeMap<ActorId, Sender<CallSignal>>,
    rooms: BTreeMap<CallId, BTreeSet<ActorId>>,
}

impl SignalingHub {
    pub fn connect(&mut self, actor_id: ActorId) -> SignalSubscription {
        let (sender, receiver) = mpsc::channel();
        self.clients.insert(actor_id.clone(), sender);
        SignalSubscription { actor_id, receiver }
    }

    pub fn disconnect(&mut self, actor_id: &ActorId) {
        self.clients.remove(actor_id);
        self.rooms.retain(|_, members| {
            members.remove(actor_id);
            !members.is_empty()
        });
    }

    pub fn snapshot(&self) -> SignalingSnapshot {
        SignalingSnapshot {
            connected_actor_ids: self.clients.keys().cloned().collect(),
            rooms: self.rooms.clone(),
        }
    }

    pub fn route(
        &mut self,
        authenticated_actor_id: &ActorId,
        signal: CallSignal,
    ) -> Result<usize, SignalingError> {
        validate_sender(authenticated_actor_id, &signal)?;
        let call_id = signal_call_id(&signal).clone();

        if let CallSignal::Invite {
            from, participants, ..
        } = &signal
        {
            let room = self.rooms.entry(call_id.clone()).or_default();
            room.insert(from.clone());
            room.extend(participants.iter().cloned());
        }

        let targets = match &signal {
            CallSignal::Invite {
                from, participants, ..
            } => participants
                .iter()
                .filter(|actor_id| *actor_id != from)
                .cloned()
                .collect::<BTreeSet<_>>(),
            CallSignal::SdpOffer { to, .. } | CallSignal::SdpAnswer { to, .. } => {
                BTreeSet::from([to.clone()])
            }
            CallSignal::IceCandidate { to: Some(to), .. } => BTreeSet::from([to.clone()]),
            CallSignal::IceCandidate { to: None, .. }
            | CallSignal::Ringing { .. }
            | CallSignal::Accept { .. }
            | CallSignal::Decline { .. }
            | CallSignal::MediaState { .. }
            | CallSignal::Speaking { .. }
            | CallSignal::Hangup { .. } => self
                .rooms
                .get(&call_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|actor_id| actor_id != authenticated_actor_id)
                .collect(),
        };

        if matches!(signal, CallSignal::Accept { .. }) {
            self.rooms
                .entry(call_id.clone())
                .or_default()
                .insert(authenticated_actor_id.clone());
        }

        let mut delivered = 0usize;
        let mut stale = Vec::new();
        for target in targets {
            let Some(sender) = self.clients.get(&target) else {
                continue;
            };
            if sender.send(signal.clone()).is_ok() {
                delivered += 1;
            } else {
                stale.push(target);
            }
        }
        for actor_id in stale {
            self.disconnect(&actor_id);
        }

        if matches!(signal, CallSignal::Hangup { .. }) {
            if let Some(room) = self.rooms.get_mut(&call_id) {
                room.remove(authenticated_actor_id);
                if room.len() < 2 {
                    self.rooms.remove(&call_id);
                }
            }
        }

        Ok(delivered)
    }
}

fn signal_call_id(signal: &CallSignal) -> &CallId {
    match signal {
        CallSignal::Invite { call_id, .. }
        | CallSignal::Ringing { call_id, .. }
        | CallSignal::Accept { call_id, .. }
        | CallSignal::Decline { call_id, .. }
        | CallSignal::SdpOffer { call_id, .. }
        | CallSignal::SdpAnswer { call_id, .. }
        | CallSignal::IceCandidate { call_id, .. }
        | CallSignal::MediaState { call_id, .. }
        | CallSignal::Speaking { call_id, .. }
        | CallSignal::Hangup { call_id, .. } => call_id,
    }
}

fn validate_sender(actor_id: &ActorId, signal: &CallSignal) -> Result<(), SignalingError> {
    let declared = match signal {
        CallSignal::Invite { from, .. }
        | CallSignal::SdpOffer { from, .. }
        | CallSignal::SdpAnswer { from, .. }
        | CallSignal::IceCandidate { from, .. } => from,
        CallSignal::Ringing { actor_id, .. }
        | CallSignal::Accept { actor_id, .. }
        | CallSignal::Decline { actor_id, .. }
        | CallSignal::MediaState { actor_id, .. }
        | CallSignal::Speaking { actor_id, .. }
        | CallSignal::Hangup { actor_id, .. } => actor_id,
    };
    if declared != actor_id {
        return Err(SignalingError::SenderMismatch {
            authenticated: actor_id.clone(),
            declared: declared.clone(),
        });
    }
    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SignalingError {
    #[error("authenticated signaling actor {authenticated:?} does not match declared sender {declared:?}")]
    SenderMismatch {
        authenticated: ActorId,
        declared: ActorId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::ConversationId;
    use crate::realtime::CallKind;

    #[test]
    fn routes_invite_and_targeted_sdp_between_connected_actors() {
        let caller = ActorId::new("human:caller");
        let callee = ActorId::new("human:callee");
        let call_id = CallId("call:1".into());
        let mut hub = SignalingHub::default();
        let _caller_subscription = hub.connect(caller.clone());
        let callee_subscription = hub.connect(callee.clone());

        let delivered = hub
            .route(
                &caller,
                CallSignal::Invite {
                    call_id: call_id.clone(),
                    conversation_id: ConversationId::new("chat:1"),
                    kind: CallKind::Video,
                    from: caller.clone(),
                    participants: vec![callee.clone()],
                },
            )
            .unwrap();
        assert_eq!(delivered, 1);
        assert!(matches!(
            callee_subscription.receiver.recv().unwrap(),
            CallSignal::Invite { .. }
        ));

        let delivered = hub
            .route(
                &caller,
                CallSignal::SdpOffer {
                    call_id,
                    from: caller.clone(),
                    to: callee,
                    sdp: "v=0".into(),
                },
            )
            .unwrap();
        assert_eq!(delivered, 1);
        assert!(matches!(
            callee_subscription.receiver.recv().unwrap(),
            CallSignal::SdpOffer { .. }
        ));
    }

    #[test]
    fn rejects_sender_spoofing() {
        let caller = ActorId::new("human:caller");
        let attacker = ActorId::new("human:attacker");
        let mut hub = SignalingHub::default();
        let error = hub
            .route(
                &attacker,
                CallSignal::Ringing {
                    call_id: CallId("call:1".into()),
                    actor_id: caller.clone(),
                },
            )
            .unwrap_err();
        assert!(matches!(error, SignalingError::SenderMismatch { .. }));
    }
}
