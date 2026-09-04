//! The traded universe: the static character of each instrument, and the
//! locate → ticker map derived from it.
//!
//! Split out of [`super::simulator`] because these are *reference data*, not
//! simulation state. Nothing here holds a mutable value or draws a random
//! number, so it is shared freely by the simulator, by the CSV writer that
//! emits the locate map, and by the summary that labels its rows.

/// ITCH prices are integers scaled by 10,000.
pub const PRICE_SCALE: u32 = 10_000;

/// One cent, in ITCH price units.
pub const TICK: u32 = 100;

/// A symbol's static character: where it opens, how much it moves, how much of
/// the tape it takes, and how hard it reacts to a market-wide shock.
#[derive(Debug, Clone, Copy)]
pub struct SymbolSpec {
    pub ticker: &'static str,
    /// Opening price in ITCH units (scaled by 10,000).
    pub open_price: u32,
    /// Baseline standard deviation of a 10 ms price step, in ticks.
    pub tick_sigma: f64,
    /// Baseline share of the tape (relative, not normalised).
    pub weight: f64,
    /// Sensitivity to the market-wide shock: 0 shrugs it off, 1 takes it fully.
    pub shock_beta: f64,
}

/// The universe. Eight names spanning three orders of magnitude in price and a
/// wide spread of volatility, so a receiver plotting them has something to see.
pub const SYMBOLS: [SymbolSpec; 8] = [
    SymbolSpec {
        ticker: "AAPL",
        open_price: 150_0000,
        tick_sigma: 0.10,
        weight: 1.00,
        shock_beta: 0.6,
    },
    SymbolSpec {
        ticker: "MSFT",
        open_price: 380_0000,
        tick_sigma: 0.14,
        weight: 0.85,
        shock_beta: 0.5,
    },
    SymbolSpec {
        ticker: "NVDA",
        open_price: 120_0000,
        tick_sigma: 0.22,
        weight: 1.30,
        shock_beta: 1.0,
    },
    SymbolSpec {
        ticker: "TSLA",
        open_price: 250_0000,
        tick_sigma: 0.30,
        weight: 1.10,
        shock_beta: 0.9,
    },
    SymbolSpec {
        ticker: "AMZN",
        open_price: 175_0000,
        tick_sigma: 0.13,
        weight: 0.70,
        shock_beta: 0.5,
    },
    SymbolSpec {
        ticker: "SPY",
        open_price: 500_0000,
        tick_sigma: 0.08,
        weight: 1.60,
        shock_beta: 0.3,
    },
    SymbolSpec {
        ticker: "GME",
        open_price: 25_0000,
        tick_sigma: 0.45,
        weight: 0.45,
        shock_beta: 1.4,
    },
    SymbolSpec {
        ticker: "F",
        open_price: 12_0000,
        tick_sigma: 0.06,
        weight: 0.60,
        shock_beta: 0.2,
    },
];

/// The locate → ticker map a receiver needs to make sense of 'D'/'E'/'X'/'U'
/// messages, which carry no ASCII symbol.
///
/// ITCH stock locates are 1-based; 0 is not a valid locate.
pub fn symbol_table() -> Vec<(u16, &'static str, u32)> {
    SYMBOLS
        .iter()
        .enumerate()
        .map(|(i, s)| ((i + 1) as u16, s.ticker, s.open_price))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_are_one_based_and_contiguous() {
        let table = symbol_table();
        assert_eq!(table.len(), SYMBOLS.len());
        for (i, (locate, ticker, open)) in table.iter().enumerate() {
            assert_eq!(*locate, (i + 1) as u16);
            assert_eq!(*ticker, SYMBOLS[i].ticker);
            assert_eq!(*open, SYMBOLS[i].open_price);
        }
    }
}
