pub use worker_real::*;

/// Compatibility wrapper for worker-rs 0.8.x. The shared payment service was
/// intentionally written in seconds from a millisecond clock; keeping this
/// adapter local avoids leaking worker-rs numeric API churn into accounting
/// code or duplicating the payment implementation.
pub struct Date(worker_real::Date);

impl Date {
    pub fn now() -> Self {
        Self(worker_real::Date::now())
    }

    pub fn as_millis(&self) -> f64 {
        self.0.as_millis() as f64
    }
}
