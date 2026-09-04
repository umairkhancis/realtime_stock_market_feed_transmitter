//! **Ring 3 — interface adapters for the human.** Everything that writes to a
//! terminal, and the only layer permitted to name a third-party crate.
//!
//! - [`cli`]     — the composition root's controller: usage text, command
//!   dispatch, and the default paths and addresses an operator gets when they
//!   pass nothing.
//! - [`console`] — renders a [`crate::application::transmit::TransmitReport`]
//!   and implements [`crate::application::ports::TransmitObserver`] so that the
//!   send loop can report progress without knowing stdout exists.
//! - [`format`]  — pure rendering helpers: hex dumps and scaled prices.
//! - [`banner`]  — the figlet/colour splash. `figlet-rs` and `colored` are
//!   reachable from this module and nowhere else in the crate, which is the
//!   whole point of putting it here.
//! - [`summary`] — the feed report the `summary` command prints.

pub mod banner;
pub mod cli;
pub mod console;
pub mod format;
pub mod summary;
