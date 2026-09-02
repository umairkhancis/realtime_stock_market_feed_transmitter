//! CSV ground truth for a generated feed.
//!
//! Slice 2 asks for the 100,000 messages on disk as CSV. The point is not
//! archival — it is that the receiver's questions ("what was NVDA's spread at
//! t+5.5s?", "how many messages did we lose between 4s and 6s?") need an answer
//! key. This file is that key: row *n* is the message carried by datagram *n*.
//!
//! One column is not a wire field. `seq` is the message's index in the file,
//! written so a human (or a diff) can line the CSV up against a receiver's log
//! without counting. It is deliberately *not* encoded into the datagram —
//! sequence numbers belong to a session layer that slice 2 does not have yet
//! (see `docs/session-layer.md`), and smuggling one into the ITCH payload would
//! model the pipe into the cargo.
//!
//! Only 'A' and 'F' carry an ASCII ticker, so the `stock` column is empty for
//! every other type — exactly as on the wire. [`write_symbol_table`] emits the
//! locate → ticker map a receiver needs to fill that in.

use std::fmt;
use std::io::{self, BufRead, Write};

use crate::market::MarketSimulator;
use crate::model::{
    pack_itch_timestamp, unpack_stock_symbol, ItchAddOrder, ItchAddOrderAttributed, ItchMessage,
    ItchOrderCancel, ItchOrderDelete, ItchOrderExecuted, ItchOrderExecutedWithPrice,
    ItchOrderReplace,
};

/// The header row, and the authority on column order.
pub const HEADER: &str = "seq,timestamp_ns,msg_type,stock_locate,tracking_number,stock,\
order_ref,new_order_ref,side,shares,price,match_number,printable,attribution";

const COLUMNS: usize = 14;

#[derive(Debug)]
pub enum FeedError {
    Io(io::Error),
    /// A row did not have [`COLUMNS`] fields.
    WrongColumnCount { line: u64, expected: usize, got: usize },
    /// A field was missing, malformed, or out of range.
    BadField { line: u64, column: &'static str, value: String },
    /// The type byte in column 2 is not one we speak.
    UnknownMessageType { line: u64, value: String },
    /// The header row is absent or renamed — refuse rather than silently
    /// misreading a file whose columns moved.
    BadHeader { got: String },
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedError::Io(e) => write!(f, "{e}"),
            FeedError::WrongColumnCount { line, expected, got } => {
                write!(f, "line {line}: expected {expected} columns, got {got}")
            }
            FeedError::BadField { line, column, value } => {
                write!(f, "line {line}: bad value {value:?} in column {column}")
            }
            FeedError::UnknownMessageType { line, value } => {
                write!(f, "line {line}: unknown message type {value:?}")
            }
            FeedError::BadHeader { got } => {
                write!(f, "not a feed CSV: expected header\n  {HEADER}\ngot\n  {got}")
            }
        }
    }
}

impl std::error::Error for FeedError {}

impl From<io::Error> for FeedError {
    fn from(e: io::Error) -> Self {
        FeedError::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Writes `msgs` as CSV, header included. Returns the number of rows written.
pub fn write_feed<W, I>(out: &mut W, msgs: I) -> Result<u64, FeedError>
where
    W: Write,
    I: IntoIterator<Item = ItchMessage>,
{
    writeln!(out, "{HEADER}")?;
    let mut rows = 0u64;
    let mut line = String::with_capacity(128);
    for msg in msgs {
        line.clear();
        render_row(&mut line, rows, &msg);
        out.write_all(line.as_bytes())?;
        out.write_all(b"\n")?;
        rows += 1;
    }
    Ok(rows)
}

/// The locate → ticker map. Every message carries a locate; only adds carry the
/// ticker, so without this a receiver cannot name the instrument in a delete.
pub fn write_symbol_table<W: Write>(out: &mut W) -> io::Result<()> {
    writeln!(out, "stock_locate,ticker,open_price")?;
    for (locate, ticker, open) in MarketSimulator::symbol_table() {
        writeln!(out, "{locate},{ticker},{open}")?;
    }
    Ok(())
}

fn render_row(buf: &mut String, seq: u64, msg: &ItchMessage) {
    use fmt::Write as _;
    let ts = msg.timestamp_nanos();
    let t = msg.message_type() as char;
    let locate = msg.stock_locate();

    // seq,timestamp_ns,msg_type,stock_locate,tracking_number,...
    match msg {
        ItchMessage::AddOrder(m) => {
            let _ = write!(
                buf,
                "{seq},{ts},{t},{locate},{},{},{},,{},{},{},,,",
                { m.tracking_number },
                unpack_stock_symbol(&{ m.stock }),
                { m.order_reference },
                m.buy_sell_indicator as char,
                { m.shares },
                { m.price },
            );
        }
        ItchMessage::AddOrderAttributed(m) => {
            let _ = write!(
                buf,
                "{seq},{ts},{t},{locate},{},{},{},,{},{},{},,,{}",
                { m.tracking_number },
                unpack_stock_symbol(&{ m.stock }),
                { m.order_reference },
                m.buy_sell_indicator as char,
                { m.shares },
                { m.price },
                ascii4(&{ m.attribution }),
            );
        }
        ItchMessage::OrderExecuted(m) => {
            let _ = write!(
                buf,
                "{seq},{ts},{t},{locate},{},,{},,,{},,{},,",
                { m.tracking_number },
                { m.order_reference },
                { m.shares },
                { m.match_number },
            );
        }
        ItchMessage::OrderExecutedWithPrice(m) => {
            let _ = write!(
                buf,
                "{seq},{ts},{t},{locate},{},,{},,,{},{},{},{},",
                { m.tracking_number },
                { m.order_reference },
                { m.shares },
                { m.execution_price },
                { m.match_number },
                m.printable as char,
            );
        }
        ItchMessage::OrderCancel(m) => {
            let _ = write!(
                buf,
                "{seq},{ts},{t},{locate},{},,{},,,{},,,,",
                { m.tracking_number },
                { m.order_reference },
                { m.canceled_shares },
            );
        }
        ItchMessage::OrderDelete(m) => {
            let _ = write!(
                buf,
                "{seq},{ts},{t},{locate},{},,{},,,,,,,",
                { m.tracking_number },
                { m.order_reference },
            );
        }
        ItchMessage::OrderReplace(m) => {
            let _ = write!(
                buf,
                "{seq},{ts},{t},{locate},{},,{},{},,{},{},,,",
                { m.tracking_number },
                { m.original_order_reference },
                { m.new_order_reference },
                { m.shares },
                { m.price },
            );
        }
    }
}

/// Renders a 4-byte MPID, trimming trailing spaces.
fn ascii4(field: &[u8; 4]) -> &str {
    let end = field.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
    std::str::from_utf8(&field[..end]).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Reads a whole feed CSV into memory.
///
/// Deliberately not streaming. The transmitter has to hold a 100 µs schedule;
/// pulling the next row off a disk read mid-loop would put page-cache misses
/// straight into the inter-packet gap. Read it all, encode it all, *then* start
/// the clock. At 100,000 messages this is a few megabytes.
pub fn read_feed<R: BufRead>(input: R) -> Result<Vec<ItchMessage>, FeedError> {
    let mut out = Vec::new();
    let mut lines = input.lines();

    match lines.next() {
        None => return Err(FeedError::BadHeader { got: String::from("<empty file>") }),
        Some(header) => {
            let header = header?;
            if header.trim_end_matches('\r') != HEADER {
                return Err(FeedError::BadHeader { got: header });
            }
        }
    }

    for (i, line) in lines.enumerate() {
        let line = line?;
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        // Line numbers are 1-based and include the header, so a parse error can
        // be opened at the right place in an editor.
        out.push(parse_row(line, i as u64 + 2)?);
    }
    Ok(out)
}

fn parse_row(line: &str, no: u64) -> Result<ItchMessage, FeedError> {
    let f: Vec<&str> = line.split(',').collect();
    if f.len() != COLUMNS {
        return Err(FeedError::WrongColumnCount { line: no, expected: COLUMNS, got: f.len() });
    }

    let num = |v: &str, column: &'static str| -> Result<u64, FeedError> {
        v.parse::<u64>().map_err(|_| FeedError::BadField {
            line: no,
            column,
            value: v.to_string(),
        })
    };
    let u32f = |v: &str, column: &'static str| -> Result<u32, FeedError> {
        num(v, column).and_then(|n| {
            u32::try_from(n).map_err(|_| FeedError::BadField {
                line: no,
                column,
                value: v.to_string(),
            })
        })
    };
    let u16f = |v: &str, column: &'static str| -> Result<u16, FeedError> {
        num(v, column).and_then(|n| {
            u16::try_from(n).map_err(|_| FeedError::BadField {
                line: no,
                column,
                value: v.to_string(),
            })
        })
    };
    let one_byte = |v: &str, column: &'static str| -> Result<u8, FeedError> {
        let b = v.as_bytes();
        if b.len() == 1 {
            Ok(b[0])
        } else {
            Err(FeedError::BadField { line: no, column, value: v.to_string() })
        }
    };
    let stock = |v: &str| -> Result<[u8; 8], FeedError> {
        crate::model::pack_stock_symbol(v).ok_or_else(|| FeedError::BadField {
            line: no,
            column: "stock",
            value: v.to_string(),
        })
    };

    let timestamp_bytes = pack_itch_timestamp(num(f[1], "timestamp_ns")?).ok_or_else(|| {
        FeedError::BadField { line: no, column: "timestamp_ns", value: f[1].to_string() }
    })?;
    let stock_locate = u16f(f[3], "stock_locate")?;
    let tracking_number = u16f(f[4], "tracking_number")?;
    let message_type = one_byte(f[2], "msg_type")?;

    Ok(match message_type {
        b'A' => ItchMessage::AddOrder(ItchAddOrder {
            message_type,
            stock_locate,
            tracking_number,
            timestamp_bytes,
            order_reference: num(f[6], "order_ref")?,
            buy_sell_indicator: one_byte(f[8], "side")?,
            shares: u32f(f[9], "shares")?,
            stock: stock(f[5])?,
            price: u32f(f[10], "price")?,
        }),
        b'F' => ItchMessage::AddOrderAttributed(ItchAddOrderAttributed {
            message_type,
            stock_locate,
            tracking_number,
            timestamp_bytes,
            order_reference: num(f[6], "order_ref")?,
            buy_sell_indicator: one_byte(f[8], "side")?,
            shares: u32f(f[9], "shares")?,
            stock: stock(f[5])?,
            price: u32f(f[10], "price")?,
            attribution: mpid(f[13]).ok_or_else(|| FeedError::BadField {
                line: no,
                column: "attribution",
                value: f[13].to_string(),
            })?,
        }),
        b'E' => ItchMessage::OrderExecuted(ItchOrderExecuted {
            message_type,
            stock_locate,
            tracking_number,
            timestamp_bytes,
            order_reference: num(f[6], "order_ref")?,
            shares: u32f(f[9], "shares")?,
            match_number: num(f[11], "match_number")?,
        }),
        b'C' => ItchMessage::OrderExecutedWithPrice(ItchOrderExecutedWithPrice {
            message_type,
            stock_locate,
            tracking_number,
            timestamp_bytes,
            order_reference: num(f[6], "order_ref")?,
            shares: u32f(f[9], "shares")?,
            match_number: num(f[11], "match_number")?,
            printable: one_byte(f[12], "printable")?,
            execution_price: u32f(f[10], "price")?,
        }),
        b'X' => ItchMessage::OrderCancel(ItchOrderCancel {
            message_type,
            stock_locate,
            tracking_number,
            timestamp_bytes,
            order_reference: num(f[6], "order_ref")?,
            canceled_shares: u32f(f[9], "shares")?,
        }),
        b'D' => ItchMessage::OrderDelete(ItchOrderDelete {
            message_type,
            stock_locate,
            tracking_number,
            timestamp_bytes,
            order_reference: num(f[6], "order_ref")?,
        }),
        b'U' => ItchMessage::OrderReplace(ItchOrderReplace {
            message_type,
            stock_locate,
            tracking_number,
            timestamp_bytes,
            original_order_reference: num(f[6], "order_ref")?,
            new_order_reference: num(f[7], "new_order_ref")?,
            shares: u32f(f[9], "shares")?,
            price: u32f(f[10], "price")?,
        }),
        _ => return Err(FeedError::UnknownMessageType { line: no, value: f[2].to_string() }),
    })
}

fn mpid(v: &str) -> Option<[u8; 4]> {
    let b = v.as_bytes();
    if b.len() > 4 || !b.iter().all(|c| c.is_ascii_graphic()) {
        return None;
    }
    let mut out = [b' '; 4];
    out[..b.len()].copy_from_slice(b);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{MarketConfig, MarketSimulator};

    fn round_trip(count: u64) -> (Vec<ItchMessage>, Vec<ItchMessage>) {
        let cfg = MarketConfig { count, ..Default::default() };
        let original: Vec<ItchMessage> = MarketSimulator::new(cfg).collect();
        let mut csv: Vec<u8> = Vec::new();
        let rows = write_feed(&mut csv, original.clone()).unwrap();
        assert_eq!(rows, count);
        let parsed = read_feed(csv.as_slice()).unwrap();
        (original, parsed)
    }

    /// The CSV is only ground truth if it is lossless. Every field of every
    /// variant has to survive the trip through text.
    #[test]
    fn csv_round_trips_every_message_exactly() {
        let (original, parsed) = round_trip(20_000);
        assert_eq!(parsed.len(), original.len());
        for (i, (a, b)) in original.iter().zip(parsed.iter()).enumerate() {
            assert_eq!(a, b, "row {i} did not survive the round trip");
        }
    }

    /// Which is the same as saying: the bytes on the wire are the same whether
    /// they came from the generator or from the file.
    #[test]
    fn csv_round_trip_preserves_the_encoded_bytes() {
        let (original, parsed) = round_trip(5_000);
        let mut a = [0u8; crate::codec::MAX_MESSAGE_LEN];
        let mut b = [0u8; crate::codec::MAX_MESSAGE_LEN];
        for (x, y) in original.iter().zip(parsed.iter()) {
            let na = crate::codec::encode(x, &mut a).unwrap();
            let nb = crate::codec::encode(y, &mut b).unwrap();
            assert_eq!(a[..na], b[..nb]);
        }
    }

    #[test]
    fn every_row_has_the_declared_columns() {
        let cfg = MarketConfig { count: 3_000, ..Default::default() };
        let mut csv: Vec<u8> = Vec::new();
        write_feed(&mut csv, MarketSimulator::new(cfg)).unwrap();
        let text = String::from_utf8(csv).unwrap();
        let mut lines = text.lines();
        assert_eq!(lines.next().unwrap(), HEADER);
        assert_eq!(HEADER.split(',').count(), COLUMNS);
        for (i, line) in lines.enumerate() {
            assert_eq!(
                line.split(',').count(),
                COLUMNS,
                "row {i} has the wrong column count: {line}"
            );
            assert!(!line.contains(",,,,,,,,,,,,,"), "row {i} is entirely empty: {line}");
        }
    }

    #[test]
    fn seq_column_counts_from_zero_without_gaps() {
        let cfg = MarketConfig { count: 1_000, ..Default::default() };
        let mut csv: Vec<u8> = Vec::new();
        write_feed(&mut csv, MarketSimulator::new(cfg)).unwrap();
        let text = String::from_utf8(csv).unwrap();
        for (i, line) in text.lines().skip(1).enumerate() {
            assert_eq!(line.split(',').next().unwrap(), i.to_string());
        }
    }

    #[test]
    fn rejects_a_file_whose_columns_moved() {
        let bad = "seq,timestamp_ns,msg_type\n0,1,A\n";
        assert!(matches!(read_feed(bad.as_bytes()), Err(FeedError::BadHeader { .. })));
        assert!(matches!(read_feed("".as_bytes()), Err(FeedError::BadHeader { .. })));
    }

    #[test]
    fn reports_the_line_number_of_a_bad_row() {
        let mut text = String::from(HEADER);
        text.push('\n');
        text.push_str("0,34200000000000,A,1,0,AAPL,9,,B,100,1500000,,,\n");
        text.push_str("1,34200000100000,A,1,0,AAPL,notanumber,,B,100,1500000,,,\n");
        match read_feed(text.as_bytes()) {
            Err(FeedError::BadField { line, column, .. }) => {
                assert_eq!(line, 3, "header is line 1, so the bad row is line 3");
                assert_eq!(column, "order_ref");
            }
            other => panic!("expected a BadField, got {other:?}"),
        }

        let mut short = String::from(HEADER);
        short.push_str("\n0,1,A,1,0\n");
        assert!(matches!(
            read_feed(short.as_bytes()),
            Err(FeedError::WrongColumnCount { line: 2, got: 5, .. })
        ));

        let mut unknown = String::from(HEADER);
        unknown.push_str("\n0,34200000000000,P,1,0,,9,,,,,,,\n");
        assert!(matches!(
            read_feed(unknown.as_bytes()),
            Err(FeedError::UnknownMessageType { line: 2, .. })
        ));
    }

    #[test]
    fn symbol_table_covers_every_locate_the_feed_uses() {
        let mut table: Vec<u8> = Vec::new();
        write_symbol_table(&mut table).unwrap();
        let text = String::from_utf8(table).unwrap();
        let locates: Vec<&str> = text.lines().skip(1).map(|l| l.split(',').next().unwrap()).collect();
        assert_eq!(locates, ["1", "2", "3", "4", "5", "6", "7", "8"]);

        let cfg = MarketConfig { count: 20_000, ..Default::default() };
        for msg in MarketSimulator::new(cfg) {
            assert!(locates.contains(&msg.stock_locate().to_string().as_str()));
        }
    }
}
