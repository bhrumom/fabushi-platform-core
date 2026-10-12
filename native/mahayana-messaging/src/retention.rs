use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Source-neutral conversation message-retention policy.
///
/// `None` at the owning Conversation layer means retention is disabled. A
/// present policy is always a positive number of seconds. The upper bound is
/// the representable `u32` contract; expiry conversion below still fails
/// closed on millisecond/time overflow instead of saturating into a bogus
/// timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRetentionPolicy {
    seconds: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MessageRetentionError {
    #[error("message retention must be greater than zero seconds")]
    ZeroSeconds,
    #[error("message retention expiry overflows the canonical timestamp range")]
    ExpiryOverflow,
    #[error("message retention revision is stale: expected {expected}, actual {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("message retention revision overflow")]
    RevisionOverflow,
}

impl MessageRetentionPolicy {
    pub fn new(seconds: u32) -> Result<Self, MessageRetentionError> {
        if seconds == 0 {
            return Err(MessageRetentionError::ZeroSeconds);
        }
        Ok(Self { seconds })
    }

    pub fn seconds(self) -> u32 {
        self.seconds
    }

    /// Derive immutable message expiry from authoritative server time.
    pub fn expires_at_ms(self, server_time_ms: i64) -> Result<i64, MessageRetentionError> {
        let ttl_ms = i64::from(self.seconds)
            .checked_mul(1_000)
            .ok_or(MessageRetentionError::ExpiryOverflow)?;
        server_time_ms
            .checked_add(ttl_ms)
            .ok_or(MessageRetentionError::ExpiryOverflow)
    }
}

/// Validate an optimistic retention-policy mutation and return the next
/// authoritative revision. Callers must persist the resulting revision with
/// the policy in the same transaction/event-journal settlement.
pub fn next_retention_revision(
    actual_revision: u64,
    expected_revision: u64,
) -> Result<u64, MessageRetentionError> {
    if actual_revision != expected_revision {
        return Err(MessageRetentionError::StaleRevision {
            expected: expected_revision,
            actual: actual_revision,
        });
    }
    actual_revision
        .checked_add(1)
        .ok_or(MessageRetentionError::RevisionOverflow)
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessageRetention {
    pub policy: Option<MessageRetentionPolicy>,
    pub revision: u64,
}

impl ConversationMessageRetention {
    pub fn change(
        self,
        seconds: Option<u32>,
        expected_revision: u64,
    ) -> Result<Self, MessageRetentionError> {
        let policy = seconds.map(MessageRetentionPolicy::new).transpose()?;
        let revision = next_retention_revision(self.revision, expected_revision)?;
        Ok(Self { policy, revision })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_reserved_for_disabled_policy_and_fails_closed() {
        assert_eq!(
            MessageRetentionPolicy::new(0),
            Err(MessageRetentionError::ZeroSeconds)
        );
    }

    #[test]
    fn expiry_is_derived_from_authoritative_server_time() {
        let policy = MessageRetentionPolicy::new(86_400).expect("valid policy");
        assert_eq!(
            policy.expires_at_ms(1_000).expect("valid expiry"),
            86_401_000
        );
    }

    #[test]
    fn expiry_overflow_is_rejected_instead_of_saturating() {
        let policy = MessageRetentionPolicy::new(1).expect("valid policy");
        assert_eq!(
            policy.expires_at_ms(i64::MAX),
            Err(MessageRetentionError::ExpiryOverflow)
        );
    }

    #[test]
    fn revision_update_requires_exact_expected_revision() {
        assert_eq!(next_retention_revision(7, 7), Ok(8));
        assert_eq!(
            next_retention_revision(8, 7),
            Err(MessageRetentionError::StaleRevision {
                expected: 7,
                actual: 8,
            })
        );
    }

    #[test]
    fn revision_overflow_is_rejected() {
        assert_eq!(
            next_retention_revision(u64::MAX, u64::MAX),
            Err(MessageRetentionError::RevisionOverflow)
        );
    }
}
