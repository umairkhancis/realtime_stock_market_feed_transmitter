use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::adapters::feed::{read_feed, write_feed, write_symbol_table};
use crate::adapters::formatter::{format_price, hex};
use crate::domain::codec::{ADD_ORDER_LEN, encode_add_order};
use crate::domain::market::{MarketConfig, MarketSimulator};
use crate::domain::model::{
    ItchAddOrder, ItchMessage, pack_itch_timestamp, pack_stock_symbol, unpack_stock_symbol,
};
use crate::infrastructure::transmit::{EncodedFeed, TransmitConfig, print_report, transmit};

pub const DEFAULT_DEST: &str = "192.168.252.18:9000";
pub const DEFAULT_PORT: &str = "9000";
pub const DEFAULT_CSV: &str = "data/feed.csv";
pub const DEFAULT_RATE_HZ: u64 = 10_000;

pub type Fallible = Result<(), Box<dyn std::error::Error>>;
// Commands
// ---------------------------------------------------------------------------

// ----------------------------- PUBLIC FUNCTIONS ---------------------------------------------

// Generate a synthetic feed and write it to CSV.
pub fn generate_signal() -> Fallible {
    // Prepare for writing generated feed.
    // Default config is 100,000 messages at 10,000 messages/second (100 µs apart).
    let config = MarketConfig::default();
    let out = PathBuf::from(DEFAULT_CSV);
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    println!(
        "generating {} messages, seed {:#018x}, {} ns apart",
        config.count, config.seed, config.interval_nanos
    );

    // Generate market messages in memory, then write them to CSV.
    // This is not the most memory-efficient way to do it, but it is simple and
    // the default count is small enough that it does not matter.
    // But if you want to generate a billion messages, you can do that too, and it will
    // stream them to CSV without ever holding them all in memory at once.
    // How? By using `MarketSimulator::new(config).for_each(|msg| write_feed(&mut file, std::iter::once(msg)))` instead of collecting them into a `Vec`.
    // But that is not the default because it is more complex and less convenient for the user.
    // The default is to generate a small number of messages in memory, then write them to CSV.
    let messages: Vec<ItchMessage> = MarketSimulator::new(config).collect();
    let rows = write_feed_to_csv(&out, &messages)?;
    let symbols_path = write_symbol_table_to_csv(&out)?;
    println!(
        "wrote {rows} rows to {} ({} bytes) and the locate map to {}",
        out.display(),
        fs::metadata(&out)?.len(),
        symbols_path.display(),
    );

    Ok(())
}

pub fn start_transmission() -> Fallible {
    let rate_hz = DEFAULT_RATE_HZ;
    let messages = read_feed(BufReader::new(File::open(DEFAULT_CSV)?))?;
    println!("read {} messages from {DEFAULT_CSV}", messages.len());

    let encoded = EncodedFeed::encode_all(&messages)?;
    println!(
        "encoded {} messages into {} payload bytes ({:.1} bytes/datagram average)",
        encoded.len(),
        encoded.total_bytes(),
        encoded.total_bytes() as f64 / encoded.len().max(1) as f64,
    );

    let cfg = TransmitConfig {
        dest: DEFAULT_DEST.to_string(),
        rate_hz,
        progress_every: Duration::from_secs(1),
    };

    let report = transmit(&encoded, &cfg)?;

    print_report(&report);
    Ok(())
}

pub fn summarise() -> Fallible {
    let messages = read_feed(BufReader::new(File::open(DEFAULT_CSV)?))?;
    crate::adapters::summary::summarise(&messages);
    Ok(())
}

/// Slice 1, unchanged: one hand-built Add Order as the entire payload.
pub fn transmit_one() -> Fallible {
    let dest = DEFAULT_DEST;
    let mut buf = [0u8; ADD_ORDER_LEN];
    let msg = ItchAddOrder {
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
    };

    let n = encode_add_order(&msg, &mut buf)?;
    println!(
        "ItchAddOrder  {} {} {} shares @ {}",
        char::from(msg.buy_sell_indicator),
        unpack_stock_symbol(&{ msg.stock }),
        { msg.shares },
        format_price(msg.price),
    );
    println!("payload ({n} bytes):\n{}", hex(&buf[..n]));

    // Bind to 0.0.0.0:0 — the kernel picks an ephemeral source port.
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    let sent = sock.send_to(&buf[..n], dest)?;
    if sent != n {
        return Err(format!("short send: wrote {sent} of {n} bytes").into());
    }

    println!("sent {sent} bytes {} -> {dest}", sock.local_addr()?);
    println!("(send_to succeeding means the kernel accepted the datagram, not that it arrived)");
    Ok(())
}

/// Resolves a destination, defaulting the port when only a host is given.
pub fn resolve(dest: &str) -> io::Result<SocketAddr> {
    let owned;
    let with_port = if dest.contains(':') {
        dest
    } else {
        owned = format!("{dest}:{DEFAULT_PORT}");
        &owned
    };
    with_port.to_socket_addrs()?.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("{dest} resolved to no address"),
        )
    })
}

// ----------------------------- PRIVATE FUNCTIONS ---------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

/// `data/feed.csv` -> `data/feed.symbols.csv`.
fn symbols_path(feed: &Path) -> PathBuf {
    let stem = feed
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    feed.with_file_name(format!("{stem}.symbols.csv"))
}

// Write the symbol table to a separate CSV file, beside the feed CSV.
// Why Symbol table? Because the feed CSV contains only the messages,
// and the symbol table contains the mapping from stock symbols to their packed representation.
// The receiver needs both to decode the feed correctly.
// The symbol table is written to a separate CSV file so that it can be reused across multiple feeds,
// and so that it can be updated independently of the feed.
fn write_symbol_table_to_csv(out: &PathBuf) -> Result<PathBuf, Box<dyn Error + 'static>> {
    let symbols_path = symbols_path(out);
    let mut symbols = BufWriter::new(File::create(&symbols_path)?);
    write_symbol_table(&mut symbols)?;
    symbols.flush()?;
    Ok(symbols_path)
}

/// Write the feed to CSV, returning the number of rows written.
fn write_feed_to_csv(
    out: &PathBuf,
    messages: &Vec<ItchMessage>,
) -> Result<u64, Box<dyn Error + 'static>> {
    let mut file = BufWriter::new(File::create(out)?);
    let rows = write_feed(&mut file, messages.iter().copied())?;
    file.flush()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_path_sits_beside_the_feed() {
        assert_eq!(
            symbols_path(Path::new("data/feed.csv")),
            Path::new("data/feed.symbols.csv")
        );
        assert_eq!(
            symbols_path(Path::new("run1.csv")),
            Path::new("run1.symbols.csv")
        );
    }

    #[test]
    fn resolve_supplies_the_default_port() {
        let a = resolve("127.0.0.1").unwrap();
        assert_eq!(a.port(), 9000);
        let b = resolve("127.0.0.1:1234").unwrap();
        assert_eq!(b.port(), 1234);
        assert!(resolve("not a host name at all").is_err());
    }

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
}
