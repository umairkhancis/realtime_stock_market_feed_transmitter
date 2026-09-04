//! Slice 1, unchanged: one hand-built Add Order as the entire payload.
//!
//! No envelope, no sequence numbers, no framing — see `docs/1_SLICE.md`. The
//! message is kept here rather than in the CLI because *which* message slice 1
//! sends is a fact about the slice, not about how it is invoked; and it is not
//! in [`crate::domain`] because the domain describes every legal ITCH message,
//! not the one this program happens to demonstrate.

use crate::application::Result;
use crate::application::ports::DatagramSink;
use crate::domain::message::{ItchAddOrder, pack_itch_timestamp, pack_stock_symbol};

/// The slice-1 message: AAPL, 100 shares, $150.25, at the opening bell.
pub fn slice_one_message() -> Result<ItchAddOrder> {
    Ok(ItchAddOrder {
        message_type: b'A',
        stock_locate: 7,
        tracking_number: 42,
        // 09:30:00.000000000 as nanoseconds since midnight.
        timestamp_bytes: pack_itch_timestamp(34_200_000_000_000)
            .ok_or("timestamp does not fit in 48 bits")?,
        order_reference: 1_234_567_890,
        buy_sell_indicator: b'B',
        shares: 100,
        stock: pack_stock_symbol("AAPL").ok_or("invalid stock symbol")?,
        price: 1_502_500, // $150.25, scaled by 10,000
    })
}

/// Sends one datagram, treating a partial write as an error rather than a
/// success — for UDP a short send means something is badly wrong.
pub fn send_single(sink: &impl DatagramSink, datagram: &[u8]) -> Result<usize> {
    let n = datagram.len();
    let sent = sink.send(datagram)?;
    if sent != n {
        return Err(format!("short send: wrote {sent} of {n} bytes").into());
    }
    Ok(sent)
}
