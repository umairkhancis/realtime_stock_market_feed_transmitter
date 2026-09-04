//! Enterprise business rules: the ITCH 5.0 protocol itself and the synthetic
//! market that produces messages conforming to it.
//!
//! Nothing in this layer performs I/O, allocates a socket, opens a file or
//! reads a clock. It depends on `std` and on itself, and on nothing else in
//! the crate — the innermost ring of the dependency rule.

pub mod codec;
pub mod market;
pub mod model;
pub mod rng;
