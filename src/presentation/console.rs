//! The console presenter: everything a transmit run prints.
//!
//! [`ConsoleObserver`] is the concrete side of
//! [`crate::application::ports::TransmitObserver`]. The send loop pushes facts
//! at it; it decides they become lines on stdout. Swap it for one that emits
//! JSON, or for `SilentObserver` in a test, and neither the use case nor the
//! UDP adapter changes.

use crate::application::ports::{TransmitObserver, TransmitProgress, TransmitStart};
use crate::application::transmit::TransmitReport;

/// Renders a transmit run to stdout, in the format the CLI has always used.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConsoleObserver;

impl TransmitObserver for ConsoleObserver {
    fn on_feed_loaded(&mut self, messages: usize, location: &str) {
        println!("read {messages} messages from {location}");
    }

    fn on_feed_encoded(&mut self, datagrams: usize, payload_bytes: usize) {
        println!(
            "encoded {} messages into {} payload bytes ({:.1} bytes/datagram average)",
            datagrams,
            payload_bytes,
            payload_bytes as f64 / datagrams.max(1) as f64,
        );
    }

    fn on_start(&mut self, start: &TransmitStart) {
        println!(
            "sending {} datagrams  {} -> {}  at {} msg/s ({:?} apart)",
            start.datagrams, start.local, start.dest, start.rate_hz, start.interval,
        );
    }

    fn on_progress(&mut self, p: &TransmitProgress) {
        println!(
            "  {:>5.1}s  {:>7} sent  {:>8.0} msg/s  late {:>5.1}%  mean {:>6.1}µs  max {:>7.1}µs",
            p.elapsed.as_secs_f64(),
            p.sent,
            p.sent as f64 / p.elapsed.as_secs_f64(),
            p.pacing.late_fraction() * 100.0,
            p.pacing.mean_lateness().as_nanos() as f64 / 1000.0,
            p.pacing.max_lateness.as_nanos() as f64 / 1000.0,
        );
    }
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
