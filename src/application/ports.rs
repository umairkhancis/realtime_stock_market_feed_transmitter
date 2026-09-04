//! The ports: what a use case needs from the world, stated as traits.
//!
//! Two shapes of dependency, and they are dispatched differently on purpose:
//!
//! - **Collaborators** ([`FeedStore`], [`FeedTransmitter`], [`DatagramSink`])
//!   are taken as `impl Trait` by the use cases, so each call site
//!   monomorphizes and the abstraction costs nothing at run time. This is the
//!   Rust answer to "won't the indirection be slow?" — for static collaborators
//!   chosen once at the composition root, it is free.
//! - **Output** ([`TransmitObserver`]) is taken as `&mut dyn`, because it is
//!   passed *through* the transmitter into the send loop. Making it generic
//!   would monomorphize the transport over the presenter for no benefit; the
//!   observer is called once per second, so one vtable hop is beneath notice.
//!
//! Each port carries an associated `Error` type rather than boxing, following
//! the pattern of `FromStr`, `TryFrom` and `Iterator`: the adapter keeps its own
//! concrete error (`FeedError`, `io::Error`), and the `'static + Error` bound is
//! exactly what lets a use case turn it into [`super::Result`] with `?`.

use std::time::Duration;

use crate::application::encoded_feed::EncodedFeed;
use crate::application::transmit::{PaceStats, TransmitConfig, TransmitReport};
use crate::domain::message::ItchMessage;

/// Where a feed and its locate map are kept.
///
/// Deliberately says nothing about files. The use cases never see a path, which
/// is why swapping CSV for Parquet — or for an in-memory fake in a test — needs
/// no change in [`crate::application`].
pub trait FeedStore {
    type Error: std::error::Error + 'static;

    /// Human-readable location, for output only. A path, a URL, a bucket key —
    /// the use case treats it as opaque and only ever prints it.
    fn location(&self) -> String;

    /// Reads the whole feed back.
    fn load(&self) -> Result<Vec<ItchMessage>, Self::Error>;

    /// Writes the feed and the locate → ticker map together, since a feed
    /// without its map is not readable by a receiver.
    fn save(
        &self,
        messages: &[ItchMessage],
        symbols: &[(u16, &'static str, u32)],
    ) -> Result<StoredFeed, Self::Error>;
}

/// What a [`FeedStore`] reports after a successful write.
#[derive(Debug, Clone)]
pub struct StoredFeed {
    pub rows: u64,
    pub feed_bytes: u64,
    pub feed_location: String,
    pub symbols_location: String,
}

/// Something that can put a whole encoded feed on a wire at a fixed rate.
///
/// The boundary is drawn at the *whole feed*, not at the individual datagram,
/// and that is a deliberate trade. Drawing it per-datagram would move the send
/// loop up into the use case, which is the more textbook split — but the loop
/// also owns a 100 µs deadline clock, so the use case would need a `Clock` port
/// too, and the pacing mechanism and the syscall would end up in different
/// rings while sharing one hot budget. Keeping the loop, the socket and the
/// clock together in one adapter keeps them cohesive; the use case still never
/// names `std::net`, which is the property that actually matters.
pub trait FeedTransmitter {
    type Error: std::error::Error + 'static;

    fn transmit(
        &mut self,
        feed: &EncodedFeed,
        config: &TransmitConfig,
        observer: &mut dyn TransmitObserver,
    ) -> Result<TransmitReport, Self::Error>;
}

/// A one-shot datagram send, for slice 1's single hand-built message.
pub trait DatagramSink {
    type Error: std::error::Error + 'static;

    fn local_address(&self) -> Result<String, Self::Error>;
    fn send(&self, datagram: &[u8]) -> Result<usize, Self::Error>;
}

/// The output port: how a transmit run narrates itself.
///
/// Clean Architecture calls this a *presenter* — the use case pushes facts out
/// through it instead of returning a rendered string, so the send loop can
/// report progress with no idea that stdout exists. Every method defaults to a
/// no-op so a test can implement the trait with an empty `impl` block.
pub trait TransmitObserver {
    fn on_feed_loaded(&mut self, _messages: usize, _location: &str) {}
    fn on_feed_encoded(&mut self, _datagrams: usize, _payload_bytes: usize) {}
    fn on_start(&mut self, _start: &TransmitStart) {}
    fn on_progress(&mut self, _progress: &TransmitProgress) {}
}

/// Everything known at the instant the first datagram is due.
#[derive(Debug, Clone)]
pub struct TransmitStart {
    pub datagrams: usize,
    /// Rendered by the adapter, so this layer needs no `std::net` types.
    pub local: String,
    pub dest: String,
    pub rate_hz: u64,
    pub interval: Duration,
}

/// A periodic snapshot of a run in flight.
#[derive(Debug, Clone, Copy)]
pub struct TransmitProgress {
    pub elapsed: Duration,
    pub sent: u64,
    pub pacing: PaceStats,
}

/// A [`TransmitObserver`] that says nothing. Useful in tests, and as the
/// default when `progress_every` is zero.
#[derive(Debug, Clone, Copy, Default)]
pub struct SilentObserver;

impl TransmitObserver for SilentObserver {}
