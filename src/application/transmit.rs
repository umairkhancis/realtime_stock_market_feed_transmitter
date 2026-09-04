//! The `transmit` use case, and the report it produces.
//!
//! Slice 2 fixes the correspondence at 1:1 — a message *is* a packet. That is
//! the expensive choice and the doc says why: at 36 bytes a message, saturating
//! a 1 Gbps link this way would mean ~27M packets per second, which is ~27M
//! trips across the kernel boundary per second, which is not happening. So the
//! payload rate is set by the syscall budget, not the link: 10,000 messages per
//! second, 10,000 datagrams per second, ~2.9 Mbps of ITCH on a link that could
//! carry 340× that.
//!
//! Batching many messages into one datagram is what buys the rest of the link
//! back, and it is what the session layer in `docs/session-layer.md` is for.
//! Not this slice.
//!
//! [`PaceStats`] lives here, next to the report that carries it, rather than
//! with the [`crate::infrastructure::time::pacer::Pacer`] that fills it in. It
//! is part of what a transmit run *reports*, and putting it in the inner ring
//! is what lets the pacer stay a replaceable mechanism: `infrastructure`
//! depending on `application` points inward and is allowed; the reverse would
//! not be.

use std::time::Duration;

use crate::application::Result;
use crate::application::encoded_feed::EncodedFeed;
use crate::application::ports::{FeedStore, FeedTransmitter, TransmitObserver};

/// Where datagrams go when the operator names nothing else.
pub const DEFAULT_DEST: &str = "192.168.252.18:9000";

/// The slice-2 rate: 10,000 messages/second, one datagram each.
pub const DEFAULT_RATE_HZ: u64 = 10_000;

#[derive(Debug, Clone)]
pub struct TransmitConfig {
    pub dest: String,
    /// Messages (and therefore datagrams) per second. 0 means unpaced.
    pub rate_hz: u64,
    /// Print a progress line this often. Zero disables it.
    pub progress_every: Duration,
}

impl Default for TransmitConfig {
    fn default() -> Self {
        TransmitConfig {
            dest: DEFAULT_DEST.to_string(),
            rate_hz: DEFAULT_RATE_HZ,
            progress_every: Duration::from_secs(1),
        }
    }
}

/// What the pacer actually achieved, as opposed to what it was asked for.
#[derive(Debug, Clone, Copy, Default)]
pub struct PaceStats {
    /// Deadlines waited on.
    pub waits: u64,
    /// Deadlines already in the past when we got to them.
    pub late: u64,
    /// Worst single overshoot.
    pub max_lateness: Duration,
    /// Summed overshoot, for a mean.
    total_lateness_nanos: u128,
}

impl PaceStats {
    /// Folds one completed wait in.
    ///
    /// The tally lives on the type that owns it rather than being written field
    /// by field from the pacer: `total_lateness_nanos` is an implementation
    /// detail of the mean, and a private field is what stops a caller in another
    /// ring from corrupting the invariant that it is the sum over `waits`.
    pub fn record(&mut self, lateness: Duration) {
        self.waits += 1;
        self.total_lateness_nanos += lateness.as_nanos();
        if lateness > Duration::ZERO {
            self.late += 1;
            if lateness > self.max_lateness {
                self.max_lateness = lateness;
            }
        }
    }

    pub fn mean_lateness(&self) -> Duration {
        if self.waits == 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos((self.total_lateness_nanos / self.waits as u128) as u64)
        }
    }

    /// Fraction of deadlines missed, in `[0, 1]`.
    pub fn late_fraction(&self) -> f64 {
        if self.waits == 0 {
            0.0
        } else {
            self.late as f64 / self.waits as f64
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TransmitReport {
    pub sent: u64,
    pub bytes: u64,
    /// Datagrams the kernel refused. Counted and carried on — a transmitter
    /// that dies on the first `ENOBUFS` tells you nothing about the run.
    pub send_errors: u64,
    /// Datagrams the kernel accepted only partially. Should always be zero for
    /// UDP; if it is not, something is very wrong and you want to know.
    pub short_sends: u64,
    pub elapsed: Duration,
    pub pacing: PaceStats,
}

impl TransmitReport {
    pub fn achieved_rate(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs > 0.0 {
            self.sent as f64 / secs
        } else {
            0.0
        }
    }

    /// Payload bits per second — ITCH only, no UDP/IP/Ethernet overhead.
    pub fn payload_bps(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs > 0.0 {
            self.bytes as f64 * 8.0 / secs
        } else {
            0.0
        }
    }

    /// Bits per second actually on the wire: every datagram also carries 8
    /// bytes of UDP header, 20 of IPv4, and 38 of Ethernet framing plus the
    /// interframe gap. At 36-byte payloads that overhead is nearly 2:1, which
    /// is the other half of why 1:1 message-to-packet is expensive.
    pub fn wire_bps(&self) -> f64 {
        const PER_DATAGRAM_OVERHEAD: u64 = 8 + 20 + 38;
        let secs = self.elapsed.as_secs_f64();
        if secs > 0.0 {
            (self.bytes + self.sent * PER_DATAGRAM_OVERHEAD) as f64 * 8.0 / secs
        } else {
            0.0
        }
    }
}

/// Reads a feed, encodes it, and hands it to a transport at a fixed rate.
///
/// Every collaborator is a port: nothing in this function knows that the feed
/// is a CSV file, that the transport is a UDP socket, or that the observer
/// writes to a terminal.
pub fn transmit_feed(
    store: &impl FeedStore,
    transmitter: &mut impl FeedTransmitter,
    config: &TransmitConfig,
    observer: &mut dyn TransmitObserver,
) -> Result<TransmitReport> {
    let messages = store.load()?;
    observer.on_feed_loaded(messages.len(), &store.location());

    let encoded = EncodedFeed::encode_all(&messages)?;
    observer.on_feed_encoded(encoded.len(), encoded.total_bytes());

    Ok(transmitter.transmit(&encoded, config, observer)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_arithmetic_is_right() {
        let report = TransmitReport {
            sent: 10_000,
            bytes: 300_000,
            send_errors: 0,
            short_sends: 0,
            elapsed: Duration::from_secs(1),
            pacing: PaceStats::default(),
        };
        assert!((report.achieved_rate() - 10_000.0).abs() < 1e-6);
        assert!((report.payload_bps() - 2_400_000.0).abs() < 1e-6);
        // 66 bytes of framing per datagram on top of 30 bytes of payload.
        assert!((report.wire_bps() - (300_000.0 + 660_000.0) * 8.0).abs() < 1e-6);
        // Wire cost is more than double the payload at these message sizes.
        assert!(report.wire_bps() > report.payload_bps() * 2.0);
    }

    #[test]
    fn a_zero_length_run_reports_zero_rather_than_dividing_by_zero() {
        let report = TransmitReport {
            sent: 0,
            bytes: 0,
            send_errors: 0,
            short_sends: 0,
            elapsed: Duration::ZERO,
            pacing: PaceStats::default(),
        };
        assert_eq!(report.achieved_rate(), 0.0);
        assert_eq!(report.payload_bps(), 0.0);
        assert_eq!(report.wire_bps(), 0.0);
    }
}
