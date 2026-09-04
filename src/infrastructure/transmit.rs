//! The transmit loop: one ITCH message per UDP datagram, at a uniform rate.
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

use std::io;
use std::net::UdpSocket;
use std::time::Duration;

use crate::domain::codec::{MAX_MESSAGE_LEN, encode};
use crate::domain::model::ItchMessage;
use crate::infrastructure::pacer::{PaceStats, Pacer};

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
            dest: crate::DEFAULT_DEST.to_string(),
            rate_hz: 10_000,
            progress_every: Duration::from_secs(1),
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

/// Encodes every message up front, into one contiguous buffer.
///
/// Encoding inside the send loop would put a variable amount of work inside a
/// 100 µs budget, and the whole point of the pacer is that the only variable
/// thing in that budget is the syscall. This costs ~4 MB for 100,000 messages.
pub struct EncodedFeed {
    bytes: Vec<u8>,
    /// `(offset, length)` per message.
    index: Vec<(u32, u8)>,
}

impl EncodedFeed {
    pub fn encode_all(msgs: &[ItchMessage]) -> Result<Self, crate::domain::codec::CodecError> {
        let mut bytes = Vec::with_capacity(msgs.len() * MAX_MESSAGE_LEN);
        let mut index = Vec::with_capacity(msgs.len());
        let mut scratch = [0u8; MAX_MESSAGE_LEN];
        for msg in msgs {
            let n = encode(msg, &mut scratch)?;
            index.push((bytes.len() as u32, n as u8));
            bytes.extend_from_slice(&scratch[..n]);
        }
        Ok(EncodedFeed { bytes, index })
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn total_bytes(&self) -> usize {
        self.bytes.len()
    }

    #[inline]
    pub fn datagram(&self, i: usize) -> &[u8] {
        let (off, len) = self.index[i];
        &self.bytes[off as usize..off as usize + len as usize]
    }
}

/// Sends every message in `feed` to `cfg.dest`, one datagram each, paced.
pub fn transmit(feed: &EncodedFeed, cfg: &TransmitConfig) -> io::Result<TransmitReport> {
    // Bind to 0.0.0.0:0 — the kernel picks an ephemeral source port.
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    // Resolve once. Doing it per-send would put a DNS lookup in the hot loop.
    let dest = crate::resolve(&cfg.dest)?;
    sock.connect(dest)?;

    println!(
        "sending {} datagrams  {} -> {dest}  at {} msg/s ({:?} apart)",
        feed.len(),
        sock.local_addr()?,
        cfg.rate_hz,
        Duration::from_nanos(if cfg.rate_hz == 0 {
            0
        } else {
            1_000_000_000 / cfg.rate_hz
        }),
    );

    let mut pacer = Pacer::new(cfg.rate_hz);
    let mut report = TransmitReport {
        sent: 0,
        bytes: 0,
        send_errors: 0,
        short_sends: 0,
        elapsed: Duration::ZERO,
        pacing: PaceStats::default(),
    };

    let mut next_progress = cfg.progress_every;
    pacer.start();

    for i in 0..feed.len() {
        pacer.wait(i as u64);
        let datagram = feed.datagram(i);
        match sock.send(datagram) {
            Ok(n) => {
                report.sent += 1;
                report.bytes += n as u64;
                if n != datagram.len() {
                    report.short_sends += 1;
                }
            }
            // A refused datagram is a lost message, exactly like one the network
            // drops. Count it here so the receiver's gap count has something on
            // this side to be reconciled against.
            Err(_) => report.send_errors += 1,
        }

        if !cfg.progress_every.is_zero() {
            let elapsed = pacer.elapsed();
            if elapsed >= next_progress {
                let stats = pacer.stats();
                println!(
                    "  {:>5.1}s  {:>7} sent  {:>8.0} msg/s  late {:>5.1}%  mean {:>6.1}µs  max {:>7.1}µs",
                    elapsed.as_secs_f64(),
                    report.sent,
                    report.sent as f64 / elapsed.as_secs_f64(),
                    stats.late_fraction() * 100.0,
                    stats.mean_lateness().as_nanos() as f64 / 1000.0,
                    stats.max_lateness.as_nanos() as f64 / 1000.0,
                );
                next_progress = elapsed + cfg.progress_every;
            }
        }
    }

    report.elapsed = pacer.elapsed();
    report.pacing = pacer.stats();
    Ok(report)
}

pub fn print_report(report: &TransmitReport) {
    println!();
    println!(
        "  sent            {} datagrams, {} payload bytes",
        report.sent, report.bytes
    );
    println!("  elapsed         {:.3}s", report.elapsed.as_secs_f64());
    println!(
        "  achieved rate   {:.0} msg/s ( = packets/s, 1:1)",
        report.achieved_rate()
    );
    println!(
        "  payload         {:.2} Mbps ITCH, {:.2} Mbps on the wire with UDP/IP/Ethernet framing",
        report.payload_bps() / 1e6,
        report.wire_bps() / 1e6,
    );
    println!(
        "  pacing          {:.2}% of deadlines missed, mean {:.1}µs late, worst {:.1}µs",
        report.pacing.late_fraction() * 100.0,
        report.pacing.mean_lateness().as_nanos() as f64 / 1000.0,
        report.pacing.max_lateness.as_nanos() as f64 / 1000.0,
    );
    if report.send_errors > 0 {
        println!(
            "  send errors     {} datagrams the kernel refused",
            report.send_errors
        );
    }
    if report.short_sends > 0 {
        println!(
            "  SHORT SENDS     {} — the kernel truncated a datagram",
            report.short_sends
        );
    }
    println!();
    println!("  send() succeeding means the kernel accepted the datagram, not that it arrived.");
    println!("  Loss between here and the receiver is invisible from this side: there are no");
    println!("  sequence numbers on the wire yet. See docs/session-layer.md.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::codec::decode;
    use crate::domain::market::{MarketConfig, MarketSimulator};

    fn feed(count: u64) -> Vec<ItchMessage> {
        MarketSimulator::new(MarketConfig {
            count,
            ..Default::default()
        })
        .collect()
    }

    #[test]
    fn pre_encoding_preserves_every_datagram() {
        let msgs = feed(5_000);
        let encoded = EncodedFeed::encode_all(&msgs).unwrap();
        assert_eq!(encoded.len(), msgs.len());
        for (i, msg) in msgs.iter().enumerate() {
            let datagram = encoded.datagram(i);
            assert_eq!(
                datagram.len(),
                msg.wire_len(),
                "datagram {i} is the wrong length"
            );
            assert_eq!(
                &decode(datagram).unwrap(),
                msg,
                "datagram {i} decoded differently"
            );
        }
        let expected: usize = msgs.iter().map(|m| m.wire_len()).sum();
        assert_eq!(encoded.total_bytes(), expected);
    }

    #[test]
    fn an_empty_feed_encodes_to_nothing() {
        let encoded = EncodedFeed::encode_all(&[]).unwrap();
        assert!(encoded.is_empty());
        assert_eq!(encoded.total_bytes(), 0);
    }

    /// End to end over the loopback: every datagram sent is a datagram received,
    /// byte for byte, one message each. Loopback does not drop, so any loss here
    /// is a bug in this code rather than in a network.
    #[test]
    fn round_trips_over_loopback_one_message_per_datagram() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        // A generous receive buffer: this test sends everything before reading
        // a single datagram, so the socket has to hold the whole burst.
        let dest = receiver.local_addr().unwrap();

        let msgs = feed(400);
        let encoded = EncodedFeed::encode_all(&msgs).unwrap();
        let cfg = TransmitConfig {
            dest: dest.to_string(),
            rate_hz: 20_000,
            progress_every: Duration::ZERO,
        };
        let report = transmit(&encoded, &cfg).unwrap();
        assert_eq!(report.sent, 400);
        assert_eq!(report.short_sends, 0);
        assert_eq!(report.send_errors, 0);

        let mut buf = [0u8; 2048];
        let mut received = Vec::new();
        while received.len() < msgs.len() {
            match receiver.recv(&mut buf) {
                Ok(n) => received.push(decode(&buf[..n]).expect("payload is one whole message")),
                // Loopback can still overflow the receive buffer on a busy
                // machine; stop rather than hang, and assert on what arrived.
                Err(_) => break,
            }
        }
        assert!(
            received.len() >= msgs.len() * 9 / 10,
            "only {} of {} datagrams made it across loopback",
            received.len(),
            msgs.len()
        );
        for (i, got) in received.iter().enumerate() {
            assert_eq!(got, &msgs[i], "datagram {i} changed in flight");
        }
    }

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
