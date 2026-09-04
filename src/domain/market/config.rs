//! How a simulated session is laid out in time.
//!
//! A [`MarketConfig`] is the *whole* input to the generator: a seed names a
//! market, and the two counts fix its length and cadence. Keeping it a plain
//! `Copy` value with a [`Default`] — rather than a builder or a set of loose
//! arguments — is what lets a caller write `MarketConfig { count, ..Default::default() }`
//! and lets a test state only the field it cares about.

/// 09:30:00.000000000 as nanoseconds since midnight — the opening bell.
pub const SESSION_OPEN_NANOS: u64 = 34_200_000_000_000;
/// Simulated time between consecutive messages at the slice-2 rate of
/// 10,000 messages/second.
pub const DEFAULT_INTERVAL_NANOS: u64 = 100_000;

/// The slice-2 target: 100,000 messages.
pub const DEFAULT_MESSAGE_COUNT: u64 = 100_000;
/// How the simulated session is laid out in time.
#[derive(Debug, Clone, Copy)]
pub struct MarketConfig {
    pub seed: u64,
    pub count: u64,
    /// Simulated nanoseconds between messages. At the slice-2 rate this equals
    /// the real inter-packet gap, so a receiver can compare arrival deltas
    /// against message timestamps directly.
    pub interval_nanos: u64,
}

impl Default for MarketConfig {
    fn default() -> Self {
        MarketConfig {
            seed: 0x5EED_1CE0_1D5E_ED17,
            count: DEFAULT_MESSAGE_COUNT,
            interval_nanos: DEFAULT_INTERVAL_NANOS,
        }
    }
}
