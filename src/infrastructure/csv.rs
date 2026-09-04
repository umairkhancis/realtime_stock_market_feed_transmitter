//! CSV: the on-disk shape of a feed, and the filesystem adapter over it.
//!
//! Split in two so that the *format* can be exercised without touching a disk:
//!
//! - [`serde`] reads and writes over any `BufRead`/`Write`, so its tests round
//!   trip through a `Vec<u8>` and never create a file.
//! - [`store`] is the thin part that knows about paths, directories and
//!   `File`, and is the only thing that implements
//!   [`crate::application::ports::FeedStore`].
//!
//! CSV is the archetypal *detail*: swap it for Parquet and not one ITCH message
//! changes. That is why it sits out here and the wire codec does not.

pub mod serde;
pub mod store;

pub use serde::{FeedError, HEADER, read_feed, write_feed, write_symbol_table};
pub use store::CsvFeedStore;
