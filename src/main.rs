use realtime_stock_market_feed_transmitter::formatter::dramatic_display;
use realtime_stock_market_feed_transmitter::run;
use std::process;

fn main() {
    dramatic_display("RT Transmitter");
    if let Err(e) = run() {
        eprintln!("RT Transmitter encountered an error: {e}");
        process::exit(1);
    }
}
