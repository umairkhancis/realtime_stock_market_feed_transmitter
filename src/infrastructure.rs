//! Frameworks and drivers: the parts that touch the operating system.
//!
//! `transmit` owns the UDP socket, `pacer` owns the wall clock. Both are
//! replaceable without the domain noticing, which is the point of keeping them
//! in the outermost ring.

pub mod pacer;
pub mod transmit;
