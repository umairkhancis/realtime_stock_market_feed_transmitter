//! **Ring 0 — enterprise rules.** What an ITCH feed *is*.
//!
//! Everything here would still be true if the transmitter sent over TCP, wrote
//! Parquet instead of CSV, or had no user interface at all. That is the test
//! this layer is held to, and it is enforced mechanically: no module below
//! `domain` may name `std::io`, `std::net`, `std::fs`, `std::thread`,
//! `std::time`, `println!`, or any third-party crate. See `tests/architecture.rs`.
//!
//! - [`message`] — the ITCH 5.0 records and the sum type the rest of the crate
//!   passes around, plus the smart constructors for the two field encodings
//!   that can fail ([`message::pack_itch_timestamp`], [`message::pack_stock_symbol`]).
//! - [`codec`] — the byte layout those records take on the wire.
//! - [`market`] — the rules of the synthetic market that produces them.
//! - [`rng`] — the deterministic generator the market is defined in terms of.
//!
//! **Why the codec is domain and not infrastructure.** Serialization is usually
//! a detail, and Clean Architecture would normally exile it outward. Not here:
//! ITCH's byte layout *is* the product. Ask the dependency-rule question — would
//! this code change if we swapped the transport or the on-disk format? No. Would
//! it change if NASDAQ revised the spec? Yes, and so would every use case that
//! depends on it. That makes the wire format an enterprise rule, which is also
//! why [`message::ItchMessage::wire_len`] can call into it without inverting
//! anything. The *transport* (UDP) and the *archive format* (CSV) are the
//! details, and both live outside.

pub mod codec;
pub mod market;
pub mod message;
pub mod rng;
