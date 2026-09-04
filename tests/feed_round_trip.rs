//! Cross-layer tests: the claims that are only true when two rings agree.
//!
//! These live in `tests/` rather than in a `#[cfg(test)]` module because they
//! span layers — the generator is `domain`, the CSV is `infrastructure`, and no
//! single module owns the property. Integration tests also see only the crate's
//! public API, which makes them a standing check that the layer boundaries are
//! reachable from outside without back doors.

use realtime_stock_market_feed_transmitter::domain::codec::{MAX_MESSAGE_LEN, encode};
use realtime_stock_market_feed_transmitter::domain::market::{
    MarketConfig, MarketSimulator, symbol_table,
};
use realtime_stock_market_feed_transmitter::domain::message::ItchMessage;
use realtime_stock_market_feed_transmitter::infrastructure::csv::{read_feed, write_feed};

/// The claim the usage text makes: the CSV and an in-memory generation with
/// the same seed are the same feed.
#[test]
fn csv_and_in_memory_generation_agree() {
    let cfg = MarketConfig {
        count: 4_000,
        ..Default::default()
    };
    let generated: Vec<ItchMessage> = MarketSimulator::new(cfg).collect();
    let mut csv = Vec::new();
    write_feed(&mut csv, generated.iter().copied()).unwrap();
    assert_eq!(read_feed(csv.as_slice()).unwrap(), generated);
}

/// And therefore the same bytes on the wire.
#[test]
fn a_round_tripped_feed_encodes_to_the_same_datagrams() {
    let cfg = MarketConfig {
        count: 2_000,
        ..Default::default()
    };
    let generated: Vec<ItchMessage> = MarketSimulator::new(cfg).collect();
    let mut csv = Vec::new();
    write_feed(&mut csv, generated.iter().copied()).unwrap();
    let parsed = read_feed(csv.as_slice()).unwrap();

    let (mut a, mut b) = ([0u8; MAX_MESSAGE_LEN], [0u8; MAX_MESSAGE_LEN]);
    for (x, y) in generated.iter().zip(parsed.iter()) {
        let na = encode(x, &mut a).unwrap();
        let nb = encode(y, &mut b).unwrap();
        assert_eq!(a[..na], b[..nb]);
    }
}

/// Every locate the generator emits is one the symbol table names, or a
/// receiver cannot label the instrument in a delete.
#[test]
fn the_symbol_table_covers_the_generated_feed() {
    let table = symbol_table();
    let cfg = MarketConfig {
        count: 20_000,
        ..Default::default()
    };
    for msg in MarketSimulator::new(cfg) {
        assert!(
            table
                .iter()
                .any(|(locate, ..)| *locate == msg.stock_locate()),
            "locate {} is not in the symbol table",
            msg.stock_locate()
        );
    }
}
