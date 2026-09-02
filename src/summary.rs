//! Reads a feed back and describes it — the receiver's questions, asked here
//! first so there is a known answer to check against.
//!
//! Slice 2's note says the 100,000-message CSV exists "so that we can ask
//! meaningful questions about the market behavior modulating in time at the
//! receiver end." This module asks three of them:
//!
//! - What is the message mix, and how many bytes per second does it imply?
//! - Which symbols own the tape, and does that change during the session?
//! - Does volatility move, and where?
//!
//! A receiver that has all the data will reproduce these numbers. A receiver
//! that lost datagrams will not, and the shape of the disagreement says where.

use crate::formatter::format_price;
use crate::model::ItchMessage;

/// The symbol the timeline focuses on. NVDA: mid-range volatility, the highest
/// share of the tape, and a shock beta of 1.0, so it shows the session's shape
/// more clearly than a sleepy name would.
const FOCUS_LOCATE: u16 = 3;

/// Timeline granularity, in simulated nanoseconds.
const BUCKET_NANOS: u64 = 1_000_000_000;

/// Sub-bucket for the realized-volatility estimate: ten samples per row.
const SUB_BUCKET_NANOS: u64 = 100_000_000;

pub fn summarise(msgs: &[ItchMessage]) {
    if msgs.is_empty() {
        println!("empty feed — nothing to summarise");
        return;
    }

    let first_ts = msgs[0].timestamp_nanos();
    let last_ts = msgs[msgs.len() - 1].timestamp_nanos();
    let span_nanos = last_ts.saturating_sub(first_ts);
    let span_secs = span_nanos as f64 / 1e9;
    let wire_bytes: u64 = msgs.iter().map(|m| m.wire_len() as u64).sum();

    println!("== feed ==");
    println!("  messages        {}", msgs.len());
    println!(
        "  session         {} -> {}  ({:.3}s of simulated market)",
        clock(first_ts),
        clock(last_ts),
        span_secs
    );
    println!(
        "  wire            {} bytes, {:.1} bytes/message average",
        wire_bytes,
        wire_bytes as f64 / msgs.len() as f64
    );
    if span_secs > 0.0 {
        println!(
            "  implied rate    {:.0} msg/s = {:.0} packets/s at 1:1, {:.2} Mbps of ITCH payload",
            msgs.len() as f64 / span_secs,
            msgs.len() as f64 / span_secs,
            wire_bytes as f64 * 8.0 / span_secs / 1e6,
        );
    }

    print_message_mix(msgs);
    print_symbols(msgs);
    print_timeline(msgs, first_ts);
}

fn clock(nanos: u64) -> String {
    let secs = nanos / 1_000_000_000;
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}.{:09}", nanos % 1_000_000_000)
}

fn label(t: u8) -> &'static str {
    match t {
        b'A' => "A  add order",
        b'F' => "F  add order, attributed",
        b'E' => "E  order executed",
        b'C' => "C  order executed, priced",
        b'X' => "X  order cancel (partial)",
        b'D' => "D  order delete",
        b'U' => "U  order replace",
        _ => "?  unknown",
    }
}

fn print_message_mix(msgs: &[ItchMessage]) {
    println!();
    println!("== message mix ==");
    let types = [b'A', b'F', b'E', b'C', b'X', b'D', b'U'];
    let mut counts = [0u64; 7];
    for m in msgs {
        if let Some(i) = types.iter().position(|&t| t == m.message_type()) {
            counts[i] += 1;
        }
    }
    let total = msgs.len() as f64;
    let peak = counts.iter().copied().max().unwrap_or(1).max(1);
    for (i, &t) in types.iter().enumerate() {
        let share = counts[i] as f64 / total;
        let bar = "#".repeat(((counts[i] * 40 / peak) as usize).max(if counts[i] > 0 { 1 } else { 0 }));
        println!("  {:<28} {:>7}  {:>5.2}%  {bar}", label(t), counts[i], share * 100.0);
    }
}

/// Per-symbol activity and the price range the adds imply. Only 'A' and 'F'
/// carry a price and a ticker, so the price columns are built from those alone —
/// which is exactly the constraint a receiver works under.
fn print_symbols(msgs: &[ItchMessage]) {
    println!();
    println!("== symbols ==");
    println!(
        "  {:<7} {:>8} {:>7} {:>7} {:>12} {:>12} {:>12} {:>12}",
        "ticker", "msgs", "share", "adds", "open", "last", "low", "high"
    );

    let table = crate::market::MarketSimulator::symbol_table();
    for (locate, ticker, _open) in table {
        let mut msg_count = 0u64;
        let mut adds = 0u64;
        let mut first = None;
        let mut last = 0u32;
        let mut lo = u32::MAX;
        let mut hi = 0u32;
        for m in msgs {
            if m.stock_locate() != locate {
                continue;
            }
            msg_count += 1;
            if let Some(p) = add_price(m) {
                adds += 1;
                first.get_or_insert(p);
                last = p;
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        if msg_count == 0 {
            continue;
        }
        let share = msg_count as f64 / msgs.len() as f64 * 100.0;
        match first {
            Some(f) => println!(
                "  {ticker:<7} {msg_count:>8} {share:>6.2}% {adds:>7} {:>12} {:>12} {:>12} {:>12}",
                format_price(f),
                format_price(last),
                format_price(lo),
                format_price(hi),
            ),
            None => println!("  {ticker:<7} {msg_count:>8} {share:>6.2}% {adds:>7} {:>12}", "-"),
        }
    }
    println!("  (prices are resting-order prices, not trades — the book's edges, not its mid)");
}

fn add_price(m: &ItchMessage) -> Option<u32> {
    add_side_price(m).map(|(_, p)| p)
}

fn add_side_price(m: &ItchMessage) -> Option<(u8, u32)> {
    match m {
        ItchMessage::AddOrder(a) => Some((a.buy_sell_indicator, a.price)),
        ItchMessage::AddOrderAttributed(a) => Some((a.buy_sell_indicator, a.price)),
        _ => None,
    }
}

/// Estimates the focus symbol's mid price in each [`SUB_BUCKET_NANOS`] window,
/// from the touch: the highest bid and lowest ask that were added inside it.
///
/// The obvious estimator — the mean of every add price in the window — is much
/// worse, and worth understanding why. Adds rest at a range of depths, so their
/// mean carries the depth distribution's dispersion as noise, and over 100 ms
/// that noise is the same size as the price move being measured. The touch is
/// pinned to the mid by construction, so it barely has any. A receiver
/// reconstructing this from the feed should use the same estimator.
///
/// `None` marks a window with no two-sided quote — nothing to measure.
fn focus_mids(msgs: &[ItchMessage], first_ts: u64, n_sub: usize) -> Vec<Option<f64>> {
    let mut best_bid = vec![0u32; n_sub];
    let mut best_ask = vec![u32::MAX; n_sub];
    for m in msgs {
        if m.stock_locate() != FOCUS_LOCATE {
            continue;
        }
        let Some((side, price)) = add_side_price(m) else { continue };
        let s = ((m.timestamp_nanos() - first_ts) / SUB_BUCKET_NANOS) as usize;
        if s >= n_sub {
            continue;
        }
        if side == b'B' {
            best_bid[s] = best_bid[s].max(price);
        } else {
            best_ask[s] = best_ask[s].min(price);
        }
    }
    (0..n_sub)
        .map(|s| {
            if best_bid[s] > 0 && best_ask[s] < u32::MAX {
                Some((best_bid[s] as f64 + best_ask[s] as f64) / 2.0)
            } else {
                None
            }
        })
        .collect()
}

fn executed_shares(m: &ItchMessage) -> Option<u32> {
    match m {
        ItchMessage::OrderExecuted(e) => Some(e.shares),
        ItchMessage::OrderExecutedWithPrice(e) => Some(e.shares),
        _ => None,
    }
}

/// The answer key for "does the market modulate in time?"
///
/// Per second of simulated session: how much of the tape each second carried,
/// how much of it traded, which name dominated, and a realized-volatility
/// estimate for the focus symbol built from 100 ms price samples.
fn print_timeline(msgs: &[ItchMessage], first_ts: u64) {
    println!();
    println!("== timeline ==");

    let bucket_of = |m: &ItchMessage| (m.timestamp_nanos() - first_ts) / BUCKET_NANOS;
    let n_buckets = (bucket_of(&msgs[msgs.len() - 1]) + 1) as usize;
    if n_buckets == 0 {
        return;
    }

    let n_symbols = crate::market::SYMBOLS.len();
    let mut msg_counts = vec![0u64; n_buckets];
    let mut exec_counts = vec![0u64; n_buckets];
    let mut exec_volume = vec![0u64; n_buckets];
    let mut per_symbol = vec![vec![0u64; n_symbols + 1]; n_buckets];

    let n_sub = n_buckets * (BUCKET_NANOS / SUB_BUCKET_NANOS) as usize;
    for m in msgs {
        let b = bucket_of(m) as usize;
        msg_counts[b] += 1;
        if let Some(n) = executed_shares(m) {
            exec_counts[b] += 1;
            exec_volume[b] += n as u64;
        }
        let locate = m.stock_locate() as usize;
        if locate <= n_symbols {
            per_symbol[b][locate] += 1;
        }
    }

    let mids = focus_mids(msgs, first_ts, n_sub);
    let per_bucket_subs = (BUCKET_NANOS / SUB_BUCKET_NANOS) as usize;
    let vols: Vec<f64> = (0..n_buckets)
        .map(|b| realized_vol(&mids, b * per_bucket_subs, per_bucket_subs))
        .collect();
    let peak_vol = vols.iter().cloned().fold(0.0f64, f64::max).max(f64::MIN_POSITIVE);

    let focus_ticker = crate::market::SYMBOLS[FOCUS_LOCATE as usize - 1].ticker;
    println!(
        "  {:>4} {:>8} {:>7} {:>10} {:>8}  {:<24}",
        "t+s", "msgs", "execs", "exec vol", "busiest", format!("{focus_ticker} volatility")
    );
    for b in 0..n_buckets {
        let busiest = (1..=n_symbols)
            .max_by_key(|&l| per_symbol[b][l])
            .map(|l| crate::market::SYMBOLS[l - 1].ticker)
            .unwrap_or("-");
        let bar = "#".repeat(((vols[b] / peak_vol) * 24.0).round() as usize);
        println!(
            "  {:>4} {:>8} {:>7} {:>10} {:>8}  {bar}",
            b, msg_counts[b], exec_counts[b], exec_volume[b], busiest
        );
    }
    println!();
    println!("  Volatility is the standard deviation of successive 100 ms changes in the");
    println!("  {focus_ticker} touch midpoint, scaled to the busiest second. The generator superimposes");
    println!("  an opening burst, a mid-session shock and a close ramp — if the receiver's copy");
    println!("  of this table has a different shape, it did not get all the messages.");
}

/// Standard deviation of successive differences between sub-bucket mid prices —
/// a realized-volatility estimate that is insensitive to the level of the price
/// and to how many orders happened to print in a given window.
fn realized_vol(mids: &[Option<f64>], start: usize, len: usize) -> f64 {
    let means: Vec<f64> =
        mids[start.min(mids.len())..(start + len).min(mids.len())].iter().flatten().copied().collect();
    if means.len() < 3 {
        return 0.0;
    }
    let diffs: Vec<f64> = means.windows(2).map(|w| w[1] - w[0]).collect();
    let mean = diffs.iter().sum::<f64>() / diffs.len() as f64;
    (diffs.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / diffs.len() as f64).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{MarketConfig, MarketSimulator};

    #[test]
    fn clock_renders_the_opening_bell() {
        assert_eq!(clock(34_200_000_000_000), "09:30:00.000000000");
        assert_eq!(clock(34_200_000_100_000), "09:30:00.000100000");
        assert_eq!(clock(34_260_000_000_000), "09:31:00.000000000");
    }

    #[test]
    fn realized_vol_is_zero_for_a_flat_price_and_positive_for_a_moving_one() {
        let flat: Vec<Option<f64>> = vec![Some(100.0); 10];
        assert_eq!(realized_vol(&flat, 0, 10), 0.0);

        let moving: Vec<Option<f64>> =
            (0..10).map(|i| Some(100.0 + (i % 3) as f64 * 7.0)).collect();
        assert!(realized_vol(&moving, 0, 10) > 0.0);

        // Windows with no two-sided quote are skipped, not counted as zero.
        let gappy: Vec<Option<f64>> =
            moving.iter().enumerate().map(|(i, m)| if i % 2 == 0 { *m } else { None }).collect();
        assert!(realized_vol(&gappy, 0, 10) > 0.0);

        // Too few samples to say anything.
        assert_eq!(realized_vol(&flat, 0, 2), 0.0);
        assert_eq!(realized_vol(&[], 0, 10), 0.0);
        assert_eq!(realized_vol(&flat, 50, 10), 0.0, "a start past the end must not panic");
    }

    /// The estimator has to actually see the shock, or the timeline is decoration.
    #[test]
    fn realized_vol_separates_the_shock_from_the_calm() {
        let msgs: Vec<_> =
            MarketSimulator::new(MarketConfig { count: 100_000, ..Default::default() }).collect();
        let mids = focus_mids(&msgs, msgs[0].timestamp_nanos(), 100);
        assert!(mids.iter().filter(|m| m.is_some()).count() > 90, "the touch is mostly two-sided");
        // Second 3 is calm; second 5 holds the shock at p = 0.55.
        let calm = realized_vol(&mids, 30, 10);
        let shock = realized_vol(&mids, 50, 10);
        assert!(shock > calm * 3.0, "shock {shock:.0} vs calm {calm:.0}");
        // And the estimator has to be quiet in the calm, or the bar chart is
        // noise: a 10 ms sigma of 0.22 ticks compounds to well under a dollar
        // over 100 ms.
        assert!(calm < 500.0, "calm volatility {calm:.0} is too noisy to read");
    }

    /// Summarising must not panic on the degenerate inputs — an empty feed, or
    /// one too short to fill a single bucket.
    #[test]
    fn survives_tiny_feeds() {
        summarise(&[]);
        let one: Vec<_> =
            MarketSimulator::new(MarketConfig { count: 1, ..Default::default() }).collect();
        summarise(&one);
        let few: Vec<_> =
            MarketSimulator::new(MarketConfig { count: 50, ..Default::default() }).collect();
        summarise(&few);
    }
}
