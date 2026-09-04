//! The synthetic market: the business rules that decide *what* messages exist.
//!
//! Three concerns, deliberately kept apart so that reference data and
//! configuration can be read without pulling in the generator's mutable state:
//!
//! - [`symbols`] — the traded universe and the locate map. Pure reference data.
//! - [`config`]  — how a session is laid out in time. Pure value type.
//! - [`simulator`] — the generator itself, the only part that owns state.
//!
//! The whole module is free of `std::io`, `std::net` and `std::time`: a market
//! is defined here, never delivered. Delivery is `infrastructure`'s problem.

pub mod config;
pub mod simulator;
pub mod symbols;

pub use config::{DEFAULT_INTERVAL_NANOS, DEFAULT_MESSAGE_COUNT, MarketConfig, SESSION_OPEN_NANOS};
pub use simulator::MarketSimulator;
pub use symbols::{PRICE_SCALE, SYMBOLS, SymbolSpec, TICK, symbol_table};
