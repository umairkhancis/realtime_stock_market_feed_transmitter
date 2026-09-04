//! **Ring 1 — application business rules.** What this program *does* with a
//! feed, expressed without ever naming a socket, a file, or a terminal.
//!
//! Each use case takes its collaborators as [`ports`] — traits declared here,
//! in the inner ring, and implemented out in [`crate::infrastructure`]. That is
//! the dependency inversion the whole layout exists for: `application` names
//! `FeedStore`, never `CsvFeedStore`, so the CSV adapter can be replaced (or
//! stubbed in a test) without a line of this layer changing.
//!
//! - [`ports`]         — the trait boundary: what the outside must provide.
//! - [`encoded_feed`]  — a feed pre-encoded to bytes, ready to hand to a transport.
//! - [`generate`]      — the `generate` use case.
//! - [`transmit`]      — the `transmit` use case, plus the report it produces.
//! - [`slice_one`]     — the single hand-built message of slice 1.
//!
//! There is deliberately no `summarise` use case: that command is a load
//! followed by a render, and wrapping `store.load()` in a function that adds
//! nothing would be ceremony, not architecture. The composition root calls the
//! port directly, which is what a composition root is for.

pub mod encoded_feed;
pub mod generate;
pub mod ports;
pub mod slice_one;
pub mod transmit;

/// The crate's application-level result.
///
/// A boxed trait object rather than a concrete enum, on purpose. The Rust Book's
/// I/O-project chapter draws exactly this line: a *library* consumed by code it
/// cannot see owes its callers a matchable error type (which is why
/// [`crate::domain::codec::CodecError`] and
/// [`crate::infrastructure::csv::FeedError`] are concrete enums), while an
/// application binary that only ever propagates errors to `main` and prints them
/// gains nothing from an enum it never matches on. The moment a second consumer
/// appears — a receiver crate, an FFI boundary — this becomes an `enum AppError`
/// with `#[non_exhaustive]`, and the ports' associated `Error` types are already
/// the seam that makes that a local change.
///
/// The `T = ()` default is what keeps the common `Result` spelling short while
/// still allowing `Result<StoredFeed>`; the module-qualified `application::Result`
/// naming follows `io::Result` and `fmt::Result` rather than inventing `AppResult`.
pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
