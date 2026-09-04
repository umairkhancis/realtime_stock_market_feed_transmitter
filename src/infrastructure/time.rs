//! Clocks and pacing — the mechanism half of "10,000 messages per second".
//!
//! The *policy* (what rate, what the report says about it) is application-level
//! and lives in [`crate::application::transmit`]. What lives here is the part
//! that blocks a thread: `Instant`, `thread::sleep`, and a spin loop.

pub mod pacer;
