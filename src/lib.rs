//! Dependency-free (std only) ITCH 5.0 codec and UDP transmitter.
//!
//! Slice 1 (see `docs/1_SLICE.md`): one Add Order message, alone, as the entire
//! UDP payload. No envelope, no sequence numbers, no framing.

pub mod codec;
pub mod model;
pub mod formatter;

use std::env;
use std::net::UdpSocket;

use codec::{encode_add_order, ADD_ORDER_LEN};
use model::{pack_itch_timestamp, pack_stock_symbol, unpack_stock_symbol, ItchAddOrder};
use formatter::{format_price, hex};

const DEFAULT_DEST: &str = "192.168.252.18:9000";

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut dest = env::args().nth(1).unwrap_or_else(|| DEFAULT_DEST.to_string());
    
    if !dest.contains(':') {
        dest.push_str(":9000");
    }

    let msg = ItchAddOrder {
        message_type: b'A' as _,
        stock_locate: 7,
        tracking_number: 42,
        // 09:30:00.000000000 as nanoseconds since midnight.
        timestamp_bytes: pack_itch_timestamp(34_200_000_000_000)
            .ok_or("timestamp does not fit in 48 bits")?,
        order_reference: 1_234_567_890,
        buy_sell_indicator: b'B' as _,
        shares: 100,
        stock: pack_stock_symbol("AAPL").ok_or("invalid stock symbol")?,
        price: 1_502_500, // $150.25, scaled by 10,000
    };

    let mut buf = [0u8; ADD_ORDER_LEN];
    let n = encode_add_order(&msg, &mut buf)?;

    println!("ItchAddOrder  {} {} {} shares @ {}",
        char::from(msg.buy_sell_indicator as u8),
        unpack_stock_symbol(&{ msg.stock }),
        { msg.shares },
        format_price(msg.price),
    );
    println!("payload ({n} bytes):\n{}", hex(&buf[..n]));

    // Bind to 0.0.0.0:0 — the kernel picks an ephemeral source port.
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    let sent = sock.send_to(&buf[..n], dest.as_str())?;
    if sent != n {
        return Err(format!("short send: wrote {sent} of {n} bytes").into());
    }

    println!("sent {sent} bytes {} -> {dest}", sock.local_addr()?);
    println!("(send_to succeeding means the kernel accepted the datagram, not that it arrived)");
    Ok(())
}
