use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

static NETWORK_TIME: NetworkTimeState = NetworkTimeState::new();

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NetworkTimeError {
    #[error("system clock is earlier than the Unix epoch")]
    InvalidSystemClock,
    #[error("network time arithmetic overflowed")]
    OutOfRange,
}

struct NetworkTimeState {
    mtproto_shift_seconds: AtomicI64,
    http_shift_seconds: AtomicI64,
    http_valid: AtomicBool,
}

impl NetworkTimeState {
    const fn new() -> Self {
        Self {
            mtproto_shift_seconds: AtomicI64::new(0),
            http_shift_seconds: AtomicI64::new(0),
            http_valid: AtomicBool::new(false),
        }
    }

    fn base_now_at(&self, local_unix_seconds: i64) -> Result<i64, NetworkTimeError> {
        local_unix_seconds
            .checked_add(self.mtproto_shift_seconds.load(Ordering::SeqCst))
            .ok_or(NetworkTimeError::OutOfRange)
    }

    fn http_now_at(&self, local_unix_seconds: i64) -> Result<i64, NetworkTimeError> {
        self.base_now_at(local_unix_seconds)?
            .checked_add(self.http_shift_seconds.load(Ordering::SeqCst))
            .ok_or(NetworkTimeError::OutOfRange)
    }

    fn update_mtproto_at(
        &self,
        server_unix_seconds: i64,
        local_unix_seconds: i64,
    ) -> Result<(), NetworkTimeError> {
        let shift = server_unix_seconds
            .checked_sub(local_unix_seconds)
            .ok_or(NetworkTimeError::OutOfRange)?;
        self.mtproto_shift_seconds.store(shift, Ordering::SeqCst);
        self.http_shift_seconds.store(0, Ordering::SeqCst);
        self.http_valid.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn update_http_at(
        &self,
        http_unix_seconds: i64,
        local_unix_seconds: i64,
    ) -> Result<(), NetworkTimeError> {
        let base_now = self.base_now_at(local_unix_seconds)?;
        let shift = http_unix_seconds
            .checked_sub(base_now)
            .ok_or(NetworkTimeError::OutOfRange)?;
        self.http_shift_seconds.store(shift, Ordering::SeqCst);
        self.http_valid.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn invalidate_http(&self) {
        self.http_valid.store(false, Ordering::SeqCst);
    }

    fn http_valid(&self) -> bool {
        self.http_valid.load(Ordering::SeqCst)
    }
}

fn local_unix_seconds() -> Result<i64, NetworkTimeError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| NetworkTimeError::InvalidSystemClock)?;
    i64::try_from(duration.as_secs()).map_err(|_| NetworkTimeError::OutOfRange)
}

pub fn http_time_valid() -> bool {
    NETWORK_TIME.http_valid()
}

pub fn http_time_now() -> Result<i64, NetworkTimeError> {
    NETWORK_TIME.http_now_at(local_unix_seconds()?)
}

pub fn update_http_time(http_unix_seconds: i64) -> Result<(), NetworkTimeError> {
    NETWORK_TIME.update_http_at(http_unix_seconds, local_unix_seconds()?)
}

pub fn invalidate_http_time() {
    NETWORK_TIME.invalidate_http();
}

pub(crate) fn update_mtproto_time(server_unix_seconds: i64) -> Result<(), NetworkTimeError> {
    NETWORK_TIME.update_mtproto_at(server_unix_seconds, local_unix_seconds()?)
}

#[cfg(test)]
mod contracts {
    use super::{NetworkTimeError, NetworkTimeState};

    #[test]
    fn default_http_time_tracks_uncalibrated_base_time() {
        let state = NetworkTimeState::new();
        assert!(!state.http_valid());
        assert_eq!(state.http_now_at(1_700_000_000), Ok(1_700_000_000));
    }

    #[test]
    fn http_update_is_relative_to_current_mtproto_adjusted_time() {
        let state = NetworkTimeState::new();
        state
            .update_mtproto_at(1_700_000_100, 1_700_000_000)
            .unwrap();
        state
            .update_http_at(1_700_000_130, 1_700_000_010)
            .unwrap();

        assert!(state.http_valid());
        assert_eq!(state.http_now_at(1_700_000_020), Ok(1_700_000_140));
    }

    #[test]
    fn explicit_http_invalidation_preserves_the_last_http_shift() {
        let state = NetworkTimeState::new();
        state
            .update_http_at(1_700_000_030, 1_700_000_000)
            .unwrap();
        state.invalidate_http();

        assert!(!state.http_valid());
        assert_eq!(state.http_now_at(1_700_000_010), Ok(1_700_000_040));
    }

    #[test]
    fn trusted_mtproto_update_invalidates_and_clears_http_calibration() {
        let state = NetworkTimeState::new();
        state
            .update_http_at(1_700_000_030, 1_700_000_000)
            .unwrap();
        state
            .update_mtproto_at(1_700_000_200, 1_700_000_100)
            .unwrap();

        assert!(!state.http_valid());
        assert_eq!(state.http_now_at(1_700_000_110), Ok(1_700_000_210));
    }

    #[test]
    fn later_http_observation_replaces_prior_calibration() {
        let state = NetworkTimeState::new();
        state.update_http_at(1_010, 1_000).unwrap();
        state.update_http_at(1_025, 1_005).unwrap();

        assert!(state.http_valid());
        assert_eq!(state.http_now_at(1_010), Ok(1_030));
    }

    #[test]
    fn checked_time_arithmetic_fails_closed() {
        let state = NetworkTimeState::new();
        state.update_mtproto_at(i64::MAX, 0).unwrap();
        assert_eq!(state.http_now_at(1), Err(NetworkTimeError::OutOfRange));

        let state = NetworkTimeState::new();
        state.update_mtproto_at(i64::MIN, 0).unwrap();
        assert_eq!(
            state.update_http_at(i64::MAX, 0),
            Err(NetworkTimeError::OutOfRange)
        );
    }
}
