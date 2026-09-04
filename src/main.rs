//! The composition root's outermost shell.
//!
//! Deliberately almost empty. Everything worth testing lives in the library
//! crate beside it — the split the Rust Book's I/O project recommends, so that
//! `main` is left with only the two things a binary can do that a library
//! cannot: read the process environment, and set an exit code.

use std::env;
use std::process;

use realtime_stock_market_feed_transmitter::presentation::banner::dramatic_display;
use realtime_stock_market_feed_transmitter::presentation::cli;

fn main() {
    dramatic_display("RT Transmitter");

    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(e) = cli::run(&args) {
        eprintln!("RT Transmitter encountered an error: {e}");
        process::exit(1);
    }
}
