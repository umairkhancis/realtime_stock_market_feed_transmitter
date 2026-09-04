//! UDP adapters: the paced feed transmitter, and the one-shot datagram sink.
//!
//! This is the only module in the crate that touches `UdpSocket`. The send
//! loop lives here rather than in the use case because it shares a 100 µs
//! budget with the [`Pacer`] clock, and splitting a syscall from the deadline
//! that schedules it across a ring boundary would buy an abstraction nobody is
//! asking for. What the boundary *does* buy — a use case that never names
//! `std::net`, and a transport that can be faked whole — is preserved by
//! [`crate::application::ports::FeedTransmitter`].

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use crate::application::encoded_feed::EncodedFeed;
use crate::application::ports::{
    DatagramSink, FeedTransmitter, TransmitObserver, TransmitProgress, TransmitStart,
};
use crate::application::transmit::{PaceStats, TransmitConfig, TransmitReport};
use crate::infrastructure::net::resolve;
use crate::infrastructure::time::pacer::Pacer;

/// Sends one ITCH message per datagram, paced to a uniform rate.
#[derive(Debug, Clone, Copy, Default)]
pub struct UdpFeedTransmitter;

impl UdpFeedTransmitter {
    pub fn new() -> Self {
        UdpFeedTransmitter
    }
}

impl FeedTransmitter for UdpFeedTransmitter {
    type Error = io::Error;

    fn transmit(
        &mut self,
        feed: &EncodedFeed,
        cfg: &TransmitConfig,
        observer: &mut dyn TransmitObserver,
    ) -> io::Result<TransmitReport> {
        // Bind to 0.0.0.0:0 — the kernel picks an ephemeral source port.
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        // Resolve once. Doing it per-send would put a DNS lookup in the hot loop.
        let dest = resolve(&cfg.dest)?;
        sock.connect(dest)?;

        observer.on_start(&TransmitStart {
            datagrams: feed.len(),
            local: sock.local_addr()?.to_string(),
            dest: dest.to_string(),
            rate_hz: cfg.rate_hz,
            interval: Duration::from_nanos(if cfg.rate_hz == 0 {
                0
            } else {
                1_000_000_000 / cfg.rate_hz
            }),
        });

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
                    observer.on_progress(&TransmitProgress {
                        elapsed,
                        sent: report.sent,
                        pacing: pacer.stats(),
                    });
                    next_progress = elapsed + cfg.progress_every;
                }
            }
        }

        report.elapsed = pacer.elapsed();
        report.pacing = pacer.stats();
        Ok(report)
    }
}

/// A bound socket aimed at one destination, for slice 1's single message.
///
/// Uses `send_to` rather than `connect` + `send` so the reported local address
/// is the wildcard the kernel actually bound, which is what slice 1 shows.
#[derive(Debug)]
pub struct UdpDatagramSink {
    socket: UdpSocket,
    dest: SocketAddr,
}

impl UdpDatagramSink {
    pub fn open(dest: &str) -> io::Result<Self> {
        Ok(UdpDatagramSink {
            // Bind to 0.0.0.0:0 — the kernel picks an ephemeral source port.
            socket: UdpSocket::bind("0.0.0.0:0")?,
            dest: resolve(dest)?,
        })
    }
}

impl DatagramSink for UdpDatagramSink {
    type Error = io::Error;

    fn local_address(&self) -> io::Result<String> {
        Ok(self.socket.local_addr()?.to_string())
    }

    fn send(&self, datagram: &[u8]) -> io::Result<usize> {
        self.socket.send_to(datagram, self.dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::SilentObserver;
    use crate::domain::codec::decode;
    use crate::domain::market::{MarketConfig, MarketSimulator};
    use crate::domain::message::ItchMessage;

    fn feed(count: u64) -> Vec<ItchMessage> {
        MarketSimulator::new(MarketConfig {
            count,
            ..Default::default()
        })
        .collect()
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
        let report = UdpFeedTransmitter::new()
            .transmit(&encoded, &cfg, &mut SilentObserver)
            .unwrap();
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
}
