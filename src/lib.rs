//! Dependency-free (std only) ITCH 5.0 codec, market generator and UDP
//! transmitter.
//!
//! **Slice 1** (`docs/1_SLICE.md`): one Add Order message, alone, as the entire
//! UDP payload. No envelope, no sequence numbers, no framing. Still available as
//! the `one` subcommand.
//!
//! **Slice 2**: 1:1 message-to-datagram at a uniform 10,000 messages/second,
//! carrying a synthetic market whose behaviour modulates over the session, with
//! the whole stream written to CSV as ground truth for the receiver.
//!
//! What is deliberately still missing: sequence numbers, heartbeats, session
//! identity, end-of-session — everything that would let the receiver *quantify*
//! loss rather than merely suffer it. That is a session layer, it sits between
//! the ITCH messages and the datagram, and `docs/session-layer.md` designs it.
//!
//! # Layout
//!
//! The crate is arranged in the four rings of clean architecture, outermost
//! last. Dependencies point inward only:
//!
//! ```text
//! main.rs            entry point / composition root
//!   application       use cases  ─┐
//!     adapters        translation │ each may depend on the ones
//!       domain        rules       ┘ nested inside it, never outward
//!   infrastructure    OS drivers (UDP socket, wall clock)
//! ```
//!
//! `docs/clean_arch.md` records why each module landed where it did.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod infrastructure;

// The crate's public façade. Callers get the use cases without having to know
// which ring they live in (Rust API Guidelines, C-REEXPORT).
pub use application::use_cases::{
    DEFAULT_CSV, DEFAULT_DEST, DEFAULT_PORT, DEFAULT_RATE_HZ, Fallible, generate_signal, resolve,
    start_transmission, summarise, transmit_one,
};
