//! Wall-clock abstraction. The library injects `Clock` so that tests can
//! produce deterministic timestamps and ULIDs (Principle VI).

/// Returns the current wall-clock time.
///
/// `SystemClock` is the default implementation. Tests construct a fixed-time
/// clock and pass it to `Store::open_with`.
pub trait Clock: Send + Sync {
    /// Returns the current wall-clock time as a [`jiff::Timestamp`].
    fn now(&self) -> jiff::Timestamp;
}

/// Default `Clock` implementation backed by the operating system.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> jiff::Timestamp {
        jiff::Timestamp::now()
    }
}
