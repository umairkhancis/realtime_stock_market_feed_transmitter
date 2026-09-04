//! A synthetic market that produces a *consistent* ITCH message stream.
//!
//! Slice 2 asks for messages that "simulate real market behavior." Two things
//! that means, and one it doesn't:
//!
//! - **Referential consistency.** Every 'E', 'C', 'X', 'D' and 'U' names an
//!   order reference that a preceding 'A' or 'F' introduced and that nothing has
//!   since removed. A receiver can therefore rebuild a book from the stream and
//!   never hit a dangling reference — which makes "did we lose messages?" a
//!   question the *data* can answer, not just the transport.
//! - **Modulation in time.** The tape is not stationary. Volatility is high at
//!   the open, calms, spikes hard mid-session, and ramps into the close; each
//!   symbol independently switches between trending and quiet regimes; and the
//!   share of the tape each symbol takes moves with them. There is something to
//!   find at the receiver.
//!
//! What it deliberately is *not*: a matching engine. There is no crossed-book
//! invariant, no auction, no price-time priority queue. Bids can print above
//! asks. That fidelity would cost a lot and buy nothing for a transport slice —
//! the receiver is measuring loss and latency, not running a market.
//!
//! Everything is driven by a seeded [`Rng`], so a seed names a market: the CSV
//! on disk and a stream generated in memory from the same seed are the same
//! bytes, message for message.

use crate::domain::message::{
    ItchAddOrder, ItchAddOrderAttributed, ItchMessage, ItchOrderCancel, ItchOrderDelete,
    ItchOrderExecuted, ItchOrderExecutedWithPrice, ItchOrderReplace, pack_itch_timestamp,
    pack_stock_symbol,
};
use crate::domain::rng::Rng;

use super::config::{MarketConfig, SESSION_OPEN_NANOS};
use super::symbols::{SYMBOLS, TICK};

/// Mids are re-drawn every this many messages, so the price process runs on a
/// wall clock (10 ms steps at 10k msg/s) rather than on message arrivals. A
/// busy symbol should not diffuse faster just because it is busy.
const PRICE_STEP_MESSAGES: u64 = 100;

/// How often each symbol re-draws its drift/vol/activity regime — 1 second at
/// the slice-2 rate.
const REGIME_MESSAGES: u64 = 10_000;

/// Below this many live orders in a symbol, only adds are emitted; there is
/// nothing to cancel or execute.
const MIN_LIVE_ORDERS: usize = 8;

/// Above this, only removals — otherwise the book grows without bound over a
/// long run and the live-order scan gets slow.
const MAX_LIVE_ORDERS: usize = 256;

/// Market participant identifiers used by attributed ('F') adds.
const MPIDS: [&[u8; 4]; 6] = [b"NSDQ", b"ARCA", b"BATS", b"EDGX", b"IEXG", b"CDRG"];

#[derive(Debug, Clone, Copy)]
struct LiveOrder {
    reference: u64,
    side: u8,
    shares: u32,
    price: u32,
}

#[derive(Debug, Clone)]
struct SymbolState {
    locate: u16,
    ticker: [u8; 8],
    /// Mid price in ITCH units, kept as f64 so a 0.1-tick step is not rounded
    /// away to nothing before it accumulates.
    mid: f64,
    tick_sigma: f64,
    base_weight: f64,
    shock_beta: f64,
    /// Drift per 10 ms step, in ticks. Re-drawn each regime; this is what makes
    /// a symbol trend rather than diffuse.
    drift: f64,
    vol_mult: f64,
    weight_mult: f64,
    half_spread_ticks: u32,
    /// Per-symbol order-reference sequence. See [`SymbolState::next_reference`].
    next_reference_seq: u64,
    live: Vec<LiveOrder>,
}

/// The generator. Yields exactly `config.count` messages, then stops.
#[derive(Debug, Clone)]
pub struct MarketSimulator {
    config: MarketConfig,
    rng: Rng,
    symbols: Vec<SymbolState>,
    index: u64,
    next_match_number: u64,
}

impl MarketSimulator {
    pub fn new(config: MarketConfig) -> Self {
        let symbols = SYMBOLS
            .iter()
            .enumerate()
            .map(|(i, spec)| SymbolState {
                // ITCH stock locates are 1-based; 0 is not a valid locate.
                locate: (i + 1) as u16,
                ticker: pack_stock_symbol(spec.ticker).expect("symbol table is valid ASCII"),
                mid: spec.open_price as f64,
                tick_sigma: spec.tick_sigma,
                base_weight: spec.weight,
                shock_beta: spec.shock_beta,
                drift: 0.0,
                vol_mult: 1.0,
                weight_mult: 1.0,
                half_spread_ticks: 1,
                next_reference_seq: 0,
                live: Vec::with_capacity(MAX_LIVE_ORDERS),
            })
            .collect();

        MarketSimulator {
            rng: Rng::new(config.seed),
            config,
            symbols,
            index: 0,
            next_match_number: 1,
        }
    }

    pub fn config(&self) -> MarketConfig {
        self.config
    }

    /// Session progress in `[0, 1)`.
    fn progress(&self) -> f64 {
        if self.config.count == 0 {
            0.0
        } else {
            self.index as f64 / self.config.count as f64
        }
    }

    /// The market-wide volatility multiplier — the signature a receiver should
    /// be able to recover from the tape.
    ///
    /// Three superimposed features: an opening burst that decays, a sharp
    /// mid-session shock, and a ramp into the close. Deterministic in `p`, so
    /// it is the same shape at every seed; the randomness rides on top of it.
    pub fn market_vol_multiplier(p: f64) -> f64 {
        let open_burst = 1.0 + 2.2 * (-p / 0.06).exp();
        let shock = 1.0 + 6.0 * (-((p - 0.55) / 0.035).powi(2)).exp();
        let close_ramp = 1.0 + 1.5 * (-(1.0 - p) / 0.07).exp();
        open_burst * shock * close_ramp
    }

    /// Re-draws each symbol's regime: trend, volatility level, and share of the
    /// tape. Called once per [`REGIME_MESSAGES`].
    fn roll_regimes(&mut self) {
        for sym in &mut self.symbols {
            // Three states, roughly: quiet, normal, agitated.
            let u = self.rng.unit();
            sym.vol_mult = if u < 0.25 {
                0.5
            } else if u < 0.80 {
                1.0
            } else {
                2.2
            };
            // A trend worth a fraction of a step's noise — enough to be visible
            // over a regime, not enough to dominate it.
            sym.drift = 0.35 * sym.tick_sigma * self.rng.normal();
            sym.weight_mult = 0.5 + 1.5 * self.rng.unit();
        }
    }

    /// Advances every symbol's mid by one 10 ms step. Runs for all symbols on a
    /// shared clock, independent of which symbols happen to be printing.
    fn advance_prices(&mut self) {
        let mv = Self::market_vol_multiplier(self.progress());
        for sym in &mut self.symbols {
            let sigma = sym.tick_sigma * sym.vol_mult * mv;
            let step = (sym.drift * mv + sigma * self.rng.normal()) * TICK as f64;
            sym.mid += step;
            // A price is a positive integer number of ticks. Floor at one cent
            // rather than letting a long down-regime walk the mid negative.
            let floor = TICK as f64;
            if sym.mid < floor {
                sym.mid = floor;
            }
            // Spreads widen when the market is moving.
            sym.half_spread_ticks = (1.0 + (mv - 1.0) * 1.2).round().clamp(1.0, 8.0) as u32;
        }
    }

    /// Current activity weights, shock-adjusted. High-beta names take a larger
    /// share of the tape exactly when the market is moving.
    fn activity_weights(&self, mv: f64) -> Vec<f64> {
        let excess = mv - 1.0;
        self.symbols
            .iter()
            .map(|s| s.base_weight * s.weight_mult * (1.0 + s.shock_beta * excess))
            .collect()
    }

    fn timestamp(&self) -> [u8; 6] {
        let nanos = SESSION_OPEN_NANOS + self.index * self.config.interval_nanos;
        pack_itch_timestamp(nanos)
            .expect("simulated session ran past midnight; lower --count or --interval-nanos")
    }

    /// Produces the next message, or `None` once `count` have been produced.
    pub fn next_message(&mut self) -> Option<ItchMessage> {
        if self.index >= self.config.count {
            return None;
        }
        if self.index % REGIME_MESSAGES == 0 {
            self.roll_regimes();
        }
        if self.index % PRICE_STEP_MESSAGES == 0 {
            self.advance_prices();
        }

        let mv = Self::market_vol_multiplier(self.progress());
        let weights = self.activity_weights(mv);
        let s = self.rng.weighted_index(&weights);
        let ts = self.timestamp();

        // Split the borrow: `rng` and one element of `symbols` are disjoint
        // fields, and the helpers below take them separately so this compiles.
        let rng = &mut self.rng;
        let sym = &mut self.symbols[s];

        let msg = match choose_kind(rng, sym.live.len(), mv) {
            Kind::Add => build_add(rng, sym, ts),
            Kind::AddAttributed => build_add_attributed(rng, sym, ts),
            Kind::Execute => {
                let m = build_execute(rng, sym, ts, self.next_match_number, false);
                self.next_match_number += 1;
                m
            }
            Kind::ExecuteWithPrice => {
                let m = build_execute(rng, sym, ts, self.next_match_number, true);
                self.next_match_number += 1;
                m
            }
            Kind::Cancel => build_cancel(rng, sym, ts),
            Kind::Delete => build_delete(rng, sym, ts),
            Kind::Replace => build_replace(rng, sym, ts),
        };

        self.index += 1;
        Some(msg)
    }
}

impl Iterator for MarketSimulator {
    type Item = ItchMessage;

    fn next(&mut self) -> Option<ItchMessage> {
        self.next_message()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Add,
    AddAttributed,
    Execute,
    ExecuteWithPrice,
    Cancel,
    Delete,
    Replace,
}

/// The message mix. Roughly NASDAQ-shaped: adds and deletes dominate, most
/// orders never trade, executions are the rare and interesting event.
fn choose_kind(rng: &mut Rng, live: usize, mv: f64) -> Kind {
    if live < MIN_LIVE_ORDERS {
        return if rng.chance(0.92) {
            Kind::Add
        } else {
            Kind::AddAttributed
        };
    }
    if live >= MAX_LIVE_ORDERS {
        // The book is full; only messages that remove an order are allowed.
        return if rng.chance(0.7) {
            Kind::Delete
        } else {
            Kind::Execute
        };
    }

    // Trading picks up when the market moves; resting-order churn does not.
    let aggression = 1.0 + (mv - 1.0) * 0.8;
    let weights = [
        470.0,             // A
        40.0,              // F
        95.0 * aggression, // E
        10.0 * aggression, // C
        50.0,              // X
        300.0,             // D
        35.0,              // U
    ];
    match rng.weighted_index(&weights) {
        0 => Kind::Add,
        1 => Kind::AddAttributed,
        2 => Kind::Execute,
        3 => Kind::ExecuteWithPrice,
        4 => Kind::Cancel,
        5 => Kind::Delete,
        _ => Kind::Replace,
    }
}

/// Order size. Round lots dominate; the long tail and the odd lots are what
/// make a naive `shares / 100` at the receiver wrong.
fn draw_shares(rng: &mut Rng) -> u32 {
    let u = rng.unit();
    if u < 0.55 {
        100
    } else if u < 0.85 {
        100 * rng.range(2, 5) as u32
    } else if u < 0.96 {
        100 * rng.range(6, 20) as u32
    } else {
        rng.range(1, 99) as u32
    }
}

/// Distance from the touch, in ticks. Cubed uniform: most orders join at or
/// near the best price, a few rest deep in the book.
fn draw_depth_ticks(rng: &mut Rng) -> u32 {
    let u = rng.unit();
    (u * u * u * 25.0) as u32
}

fn draw_side(rng: &mut Rng) -> u8 {
    if rng.chance(0.5) { b'B' } else { b'S' }
}

/// Turns a mid and an offset into a legal integer price: at least one tick, and
/// always an exact multiple of a tick.
fn quote_price(mid: f64, side: u8, offset_ticks: u32) -> u32 {
    let offset = (offset_ticks * TICK) as f64;
    let raw = if side == b'B' {
        mid - offset
    } else {
        mid + offset
    };
    let ticks = (raw / TICK as f64).round().max(1.0);
    // Clamp well inside u32 so a runaway walk cannot wrap the price.
    let ticks = ticks.min((u32::MAX / TICK) as f64);
    ticks as u32 * TICK
}

fn build_add(rng: &mut Rng, sym: &mut SymbolState, ts: [u8; 6]) -> ItchMessage {
    let (reference, side, shares, price) = new_resting_order(rng, sym);
    sym.live.push(LiveOrder {
        reference,
        side,
        shares,
        price,
    });
    ItchMessage::AddOrder(ItchAddOrder {
        message_type: b'A',
        stock_locate: sym.locate,
        tracking_number: 0,
        timestamp_bytes: ts,
        order_reference: reference,
        buy_sell_indicator: side,
        shares,
        stock: sym.ticker,
        price,
    })
}

fn build_add_attributed(rng: &mut Rng, sym: &mut SymbolState, ts: [u8; 6]) -> ItchMessage {
    let (reference, side, shares, price) = new_resting_order(rng, sym);
    sym.live.push(LiveOrder {
        reference,
        side,
        shares,
        price,
    });
    let mpid = *MPIDS[rng.below(MPIDS.len() as u64) as usize];
    ItchMessage::AddOrderAttributed(ItchAddOrderAttributed {
        message_type: b'F',
        stock_locate: sym.locate,
        tracking_number: 0,
        timestamp_bytes: ts,
        order_reference: reference,
        buy_sell_indicator: side,
        shares,
        stock: sym.ticker,
        price,
        attribution: mpid,
    })
}

/// Draws the side, size and resting price for a new order, and allocates its
/// reference.
fn new_resting_order(rng: &mut Rng, sym: &mut SymbolState) -> (u64, u8, u32, u32) {
    let side = draw_side(rng);
    let shares = draw_shares(rng);
    let price = quote_price(sym.mid, side, sym.half_spread_ticks + draw_depth_ticks(rng));
    (sym.next_reference(), side, shares, price)
}

impl SymbolState {
    /// Order references are interleaved by locate — `seq * 8 + locate` — so
    /// each symbol allocates from its own space with no shared counter to
    /// borrow across the split mutable borrow in `next_message`, and no two
    /// symbols can ever collide. As a bonus, `reference % 8` recovers the book.
    fn next_reference(&mut self) -> u64 {
        self.next_reference_seq += 1;
        self.next_reference_seq * SYMBOLS.len() as u64 + self.locate as u64
    }
}

/// Picks the index of the best-priced live order on `side`: highest bid, lowest
/// ask. Returns `None` if that side of the book is empty.
fn best_index(sym: &SymbolState, side: u8) -> Option<usize> {
    let mut best: Option<(usize, u32)> = None;
    for (i, o) in sym.live.iter().enumerate() {
        if o.side != side {
            continue;
        }
        best = match best {
            None => Some((i, o.price)),
            Some((bi, bp)) => {
                let better = if side == b'B' {
                    o.price > bp
                } else {
                    o.price < bp
                };
                if better {
                    Some((i, o.price))
                } else {
                    Some((bi, bp))
                }
            }
        };
    }
    best.map(|(i, _)| i)
}

/// Executions hit the top of the book; cancels and deletes happen anywhere.
fn pick_for_execution(rng: &mut Rng, sym: &SymbolState) -> usize {
    let side = draw_side(rng);
    best_index(sym, side)
        .or_else(|| best_index(sym, if side == b'B' { b'S' } else { b'B' }))
        .unwrap_or(0)
}

fn build_execute(
    rng: &mut Rng,
    sym: &mut SymbolState,
    ts: [u8; 6],
    match_number: u64,
    with_price: bool,
) -> ItchMessage {
    let i = pick_for_execution(rng, sym);
    let order = sym.live[i];
    // Full fill, or a round-lot slice of what is left.
    let executed = if order.shares <= 100 || rng.chance(0.35) {
        order.shares
    } else {
        let max_lots = (order.shares / 100).max(1);
        (100 * rng.range(1, max_lots as u64) as u32).min(order.shares)
    };
    if executed >= order.shares {
        sym.live.swap_remove(i);
    } else {
        sym.live[i].shares -= executed;
    }

    if with_price {
        // 'C' exists for prints that did not happen at the order's display
        // price. Move it a tick or two and mark most of them printable.
        let slide = rng.range(1, 3) as u32 * TICK;
        let execution_price = if order.side == b'B' {
            order.price.saturating_sub(slide).max(TICK)
        } else {
            order.price.saturating_add(slide)
        };
        ItchMessage::OrderExecutedWithPrice(ItchOrderExecutedWithPrice {
            message_type: b'C',
            stock_locate: sym.locate,
            tracking_number: 0,
            timestamp_bytes: ts,
            order_reference: order.reference,
            shares: executed,
            match_number,
            printable: if rng.chance(0.8) { b'Y' } else { b'N' },
            execution_price,
        })
    } else {
        ItchMessage::OrderExecuted(ItchOrderExecuted {
            message_type: b'E',
            stock_locate: sym.locate,
            tracking_number: 0,
            timestamp_bytes: ts,
            order_reference: order.reference,
            shares: executed,
            match_number,
        })
    }
}

fn build_cancel(rng: &mut Rng, sym: &mut SymbolState, ts: [u8; 6]) -> ItchMessage {
    let i = rng.below(sym.live.len() as u64) as usize;
    let order = sym.live[i];
    // 'X' is a *partial* cancel — the order stays live. Removing the last share
    // with an 'X' would leave a phantom order in a receiver's book; ITCH uses
    // 'D' for that, so fall through to a delete when there is nothing to trim.
    if order.shares < 2 {
        sym.live.swap_remove(i);
        return ItchMessage::OrderDelete(ItchOrderDelete {
            message_type: b'D',
            stock_locate: sym.locate,
            tracking_number: 0,
            timestamp_bytes: ts,
            order_reference: order.reference,
        });
    }
    let canceled = rng.range(1, (order.shares - 1) as u64) as u32;
    sym.live[i].shares -= canceled;
    ItchMessage::OrderCancel(ItchOrderCancel {
        message_type: b'X',
        stock_locate: sym.locate,
        tracking_number: 0,
        timestamp_bytes: ts,
        order_reference: order.reference,
        canceled_shares: canceled,
    })
}

fn build_delete(rng: &mut Rng, sym: &mut SymbolState, ts: [u8; 6]) -> ItchMessage {
    let i = rng.below(sym.live.len() as u64) as usize;
    let order = sym.live.swap_remove(i);
    ItchMessage::OrderDelete(ItchOrderDelete {
        message_type: b'D',
        stock_locate: sym.locate,
        tracking_number: 0,
        timestamp_bytes: ts,
        order_reference: order.reference,
    })
}

fn build_replace(rng: &mut Rng, sym: &mut SymbolState, ts: [u8; 6]) -> ItchMessage {
    let i = rng.below(sym.live.len() as u64) as usize;
    let old = sym.live.swap_remove(i);
    let shares = draw_shares(rng);
    let price = quote_price(
        sym.mid,
        old.side,
        sym.half_spread_ticks + draw_depth_ticks(rng),
    );
    let reference = sym.next_reference();
    sym.live.push(LiveOrder {
        reference,
        side: old.side,
        shares,
        price,
    });
    ItchMessage::OrderReplace(ItchOrderReplace {
        message_type: b'U',
        stock_locate: sym.locate,
        tracking_number: 0,
        timestamp_bytes: ts,
        original_order_reference: old.reference,
        new_order_reference: reference,
        shares,
        price,
    })
}

#[cfg(test)]
mod tests {
    use super::super::symbol_table;
    use super::*;
    use crate::domain::message::unpack_stock_symbol;
    use std::collections::{HashMap, HashSet};

    fn generate(count: u64) -> Vec<ItchMessage> {
        MarketSimulator::new(MarketConfig {
            count,
            ..Default::default()
        })
        .collect()
    }

    #[test]
    fn produces_exactly_the_requested_count() {
        assert_eq!(generate(0).len(), 0);
        assert_eq!(generate(1).len(), 1);
        assert_eq!(generate(25_000).len(), 25_000);
    }

    #[test]
    fn a_seed_names_a_market() {
        let a: Vec<_> = MarketSimulator::new(MarketConfig {
            seed: 99,
            count: 5_000,
            ..Default::default()
        })
        .collect();
        let b: Vec<_> = MarketSimulator::new(MarketConfig {
            seed: 99,
            count: 5_000,
            ..Default::default()
        })
        .collect();
        let c: Vec<_> = MarketSimulator::new(MarketConfig {
            seed: 100,
            count: 5_000,
            ..Default::default()
        })
        .collect();
        assert_eq!(a, b, "same seed must reproduce the stream exactly");
        assert_ne!(a, c);
    }

    /// The load-bearing invariant: a receiver can replay this stream into a book
    /// and never see a reference it does not already hold.
    ///
    /// Run across several seeds, not just the default — a generator that only
    /// stays consistent on the seed its tests use is not consistent.
    #[test]
    fn every_reference_is_live_when_used() {
        replay_and_check(&generate(100_000));
        for seed in [0u64, 1, 7, 0xDEAD_BEEF, u64::MAX] {
            let cfg = MarketConfig {
                seed,
                count: 30_000,
                ..Default::default()
            };
            replay_and_check(&MarketSimulator::new(cfg).collect::<Vec<_>>());
        }
    }

    fn replay_and_check(msgs: &[ItchMessage]) {
        let mut book: HashMap<u64, (u16, u32)> = HashMap::new();
        let mut ever_seen: HashSet<u64> = HashSet::new();
        for (i, &msg) in msgs.iter().enumerate() {
            let where_ = || format!("at message {i} ({})", msg.message_type() as char);
            match msg {
                ItchMessage::AddOrder(m) => {
                    let r = m.order_reference;
                    assert!(ever_seen.insert(r), "reference {r} reused {}", where_());
                    book.insert(r, (m.stock_locate, m.shares));
                }
                ItchMessage::AddOrderAttributed(m) => {
                    let r = m.order_reference;
                    assert!(ever_seen.insert(r), "reference {r} reused {}", where_());
                    book.insert(r, (m.stock_locate, m.shares));
                }
                ItchMessage::OrderExecuted(m) => {
                    let (r, n) = (m.order_reference, m.shares);
                    let e = book
                        .get_mut(&r)
                        .unwrap_or_else(|| panic!("dangling {r} {}", where_()));
                    assert_eq!(
                        e.0,
                        { m.stock_locate },
                        "executed on the wrong book {}",
                        where_()
                    );
                    assert!(n > 0 && n <= e.1, "over-execution of {r} {}", where_());
                    e.1 -= n;
                    if e.1 == 0 {
                        book.remove(&r);
                    }
                }
                ItchMessage::OrderExecutedWithPrice(m) => {
                    let (r, n) = (m.order_reference, m.shares);
                    let e = book
                        .get_mut(&r)
                        .unwrap_or_else(|| panic!("dangling {r} {}", where_()));
                    assert!(n > 0 && n <= e.1, "over-execution of {r} {}", where_());
                    e.1 -= n;
                    if e.1 == 0 {
                        book.remove(&r);
                    }
                }
                ItchMessage::OrderCancel(m) => {
                    let (r, n) = (m.order_reference, m.canceled_shares);
                    let e = book
                        .get_mut(&r)
                        .unwrap_or_else(|| panic!("dangling {r} {}", where_()));
                    // A cancel never empties an order; that is what 'D' is for.
                    assert!(
                        n > 0 && n < e.1,
                        "cancel of {n}/{} on {r} {}",
                        e.1,
                        where_()
                    );
                    e.1 -= n;
                }
                ItchMessage::OrderDelete(m) => {
                    let r = m.order_reference;
                    assert!(book.remove(&r).is_some(), "dangling {r} {}", where_());
                }
                ItchMessage::OrderReplace(m) => {
                    let (old, new) = (m.original_order_reference, m.new_order_reference);
                    let prev = book
                        .remove(&old)
                        .unwrap_or_else(|| panic!("dangling {old} {}", where_()));
                    assert!(ever_seen.insert(new), "reference {new} reused {}", where_());
                    assert_eq!(prev.0, { m.stock_locate });
                    book.insert(new, (m.stock_locate, m.shares));
                }
            }
        }
        assert!(
            !book.is_empty(),
            "the session should end with resting orders"
        );
    }

    #[test]
    fn timestamps_are_monotonic_and_match_the_emission_schedule() {
        let cfg = MarketConfig {
            count: 20_000,
            ..Default::default()
        };
        for (i, msg) in MarketSimulator::new(cfg).enumerate() {
            let expected = SESSION_OPEN_NANOS + i as u64 * cfg.interval_nanos;
            assert_eq!(msg.timestamp_nanos(), expected, "at message {i}");
            assert!(msg.timestamp_nanos() < 86_400_000_000_000, "past midnight");
        }
    }

    #[test]
    fn all_seven_message_types_appear_in_realistic_proportions() {
        let mut counts: HashMap<u8, usize> = HashMap::new();
        let msgs = generate(100_000);
        for m in &msgs {
            *counts.entry(m.message_type()).or_default() += 1;
        }
        for t in [b'A', b'F', b'E', b'C', b'X', b'D', b'U'] {
            assert!(
                counts.get(&t).copied().unwrap_or(0) > 0,
                "no {} messages",
                t as char
            );
        }
        let total = msgs.len() as f64;
        let share = |t: u8| counts.get(&t).copied().unwrap_or(0) as f64 / total;
        // Adds must outnumber removals or the book drains; deletes must be the
        // bulk of the rest or it grows without bound.
        assert!(
            share(b'A') > 0.35 && share(b'A') < 0.60,
            "A share {}",
            share(b'A')
        );
        assert!(
            share(b'D') > 0.20 && share(b'D') < 0.45,
            "D share {}",
            share(b'D')
        );
        assert!(share(b'C') < 0.05, "C should be rare, got {}", share(b'C'));
    }

    #[test]
    fn prices_stay_positive_and_tick_aligned() {
        for msg in generate(100_000) {
            let price = match msg {
                ItchMessage::AddOrder(m) => Some(m.price),
                ItchMessage::AddOrderAttributed(m) => Some(m.price),
                ItchMessage::OrderExecutedWithPrice(m) => Some(m.execution_price),
                ItchMessage::OrderReplace(m) => Some(m.price),
                _ => None,
            };
            if let Some(p) = price {
                assert!(p >= TICK, "price {p} below one tick");
                assert_eq!(p % TICK, 0, "price {p} is not a whole cent");
            }
        }
    }

    #[test]
    fn shares_are_never_zero() {
        for msg in generate(50_000) {
            let n = match msg {
                ItchMessage::AddOrder(m) => Some(m.shares),
                ItchMessage::AddOrderAttributed(m) => Some(m.shares),
                ItchMessage::OrderExecuted(m) => Some(m.shares),
                ItchMessage::OrderExecutedWithPrice(m) => Some(m.shares),
                ItchMessage::OrderCancel(m) => Some(m.canceled_shares),
                ItchMessage::OrderReplace(m) => Some(m.shares),
                ItchMessage::OrderDelete(_) => None,
            };
            if let Some(n) = n {
                assert!(n > 0, "a zero-share message is meaningless");
            }
        }
    }

    /// The whole point of slice 2's CSV: the tape has to *modulate*, or there is
    /// nothing to ask questions about at the receiver.
    #[test]
    fn volatility_is_higher_during_the_shock_than_in_the_calm() {
        // Realized price dispersion for one symbol, measured from its adds.
        let msgs = generate(100_000);
        let mut calm: Vec<f64> = Vec::new();
        let mut shock: Vec<f64> = Vec::new();
        for (i, m) in msgs.iter().enumerate() {
            if let ItchMessage::AddOrder(a) = m {
                if a.stock_locate != 3 {
                    continue; // NVDA: shock_beta 1.0
                }
                let p = i as f64 / msgs.len() as f64;
                if (0.25..0.35).contains(&p) {
                    calm.push({ a.price } as f64);
                } else if (0.53..0.57).contains(&p) {
                    shock.push({ a.price } as f64);
                }
            }
        }
        assert!(
            calm.len() > 50 && shock.len() > 50,
            "not enough samples: {} / {}",
            calm.len(),
            shock.len()
        );
        let sd = |v: &[f64]| {
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
        };
        let (c, s) = (sd(&calm), sd(&shock));
        assert!(
            s > c * 1.5,
            "shock sd {s:.0} is not clearly above calm sd {c:.0}"
        );
    }

    #[test]
    fn the_shock_pulls_activity_toward_high_beta_names() {
        let msgs = generate(100_000);
        let mut calm = [0usize; 9];
        let mut shock = [0usize; 9];
        for (i, m) in msgs.iter().enumerate() {
            let p = i as f64 / msgs.len() as f64;
            let locate = m.stock_locate() as usize;
            if (0.25..0.35).contains(&p) {
                calm[locate] += 1;
            } else if (0.53..0.57).contains(&p) {
                shock[locate] += 1;
            }
        }
        let frac = |c: &[usize; 9], l: usize| c[l] as f64 / c.iter().sum::<usize>() as f64;
        // GME (locate 7) has the highest shock beta; SPY (locate 6) the lowest.
        assert!(
            frac(&shock, 7) > frac(&calm, 7),
            "high-beta share did not rise: {} -> {}",
            frac(&calm, 7),
            frac(&shock, 7)
        );
        assert!(
            frac(&shock, 6) < frac(&calm, 6),
            "low-beta share did not fall: {} -> {}",
            frac(&calm, 6),
            frac(&shock, 6)
        );
    }

    #[test]
    fn adds_carry_the_ticker_and_it_matches_the_locate() {
        let table: HashMap<u16, &str> =
            symbol_table().into_iter().map(|(l, t, _)| (l, t)).collect();
        let mut checked = 0;
        for msg in generate(20_000) {
            let (locate, stock) = match msg {
                ItchMessage::AddOrder(m) => (m.stock_locate, m.stock),
                ItchMessage::AddOrderAttributed(m) => (m.stock_locate, m.stock),
                _ => continue,
            };
            assert_eq!(unpack_stock_symbol(&stock), table[&locate]);
            checked += 1;
        }
        assert!(checked > 1_000);
    }

    #[test]
    fn market_vol_multiplier_has_its_shock_where_advertised() {
        let calm = MarketSimulator::market_vol_multiplier(0.30);
        let peak = MarketSimulator::market_vol_multiplier(0.55);
        let open = MarketSimulator::market_vol_multiplier(0.0);
        let close = MarketSimulator::market_vol_multiplier(0.999);
        assert!(
            peak > calm * 5.0,
            "shock {peak} is not a shock next to {calm}"
        );
        assert!(
            open > calm * 2.0,
            "the open should be busy: {open} vs {calm}"
        );
        assert!(
            close > calm * 2.0,
            "the close should be busy: {close} vs {calm}"
        );
        assert!(calm >= 1.0);
    }
}
