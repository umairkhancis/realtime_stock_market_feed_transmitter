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
//! Four rings, dependencies pointing inward only. `docs/clean_arch.md` argues
//! every placement below and cites the Rust guidance behind it;
//! `tests/architecture.rs` enforces the arrows.
//!
//! ```text
//!   presentation ─┐                     cli, console, summary, banner, format
//!                 ├─→ application ─→ domain
//!   infrastructure┘                      ↑          ports, use cases, reports
//!        csv, udp, pacer                 └── ITCH messages, codec, market, rng
//! ```
//!
//! - [`domain`] — what an ITCH feed *is*. No I/O, no clock, no dependencies.
//! - [`application`] — what this program *does* with one, expressed over ports.
//! - [`infrastructure`] — the adapters that satisfy those ports: files, sockets,
//!   clocks.
//! - [`presentation`] — the terminal, and the composition root that wires the
//!   other three together.
//!
//! The rule that makes this more than folder-sorting: an inner ring may not
//! name an outer one. `application` declares [`application::ports::FeedStore`];
//! `infrastructure` implements it; only `presentation::cli` knows that the
//! implementation is a CSV file.

pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod presentation;
