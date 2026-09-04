//! **Ring 3 — frameworks and drivers.** How the outside world is actually
//! touched.
//!
//! Every adapter here implements a port declared in [`crate::application`], and
//! is chosen by the composition root in `main.rs`. Nothing in `domain` or
//! `application` names a module in this layer; the dependency arrows all point
//! inward, which is what makes the loopback test in [`net::udp`] a test of one
//! adapter rather than a test of the whole program.
//!
//! - [`csv`]  — the on-disk feed format, and [`csv::CsvFeedStore`], the
//!   [`crate::application::ports::FeedStore`] adapter over the filesystem.
//! - [`net`]  — address resolution and [`net::udp::UdpFeedTransmitter`], the
//!   [`crate::application::ports::FeedTransmitter`] adapter over `UdpSocket`.
//! - [`time`] — [`time::pacer::Pacer`], the hybrid sleep/spin clock the
//!   transmitter paces against.

pub mod csv;
pub mod net;
pub mod time;
