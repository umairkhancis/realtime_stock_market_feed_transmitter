//! Use cases: the four things this program can be asked to do — generate a
//! feed, transmit a feed, summarise a feed, send a single hand-built message.
//!
//! Each use case orchestrates the inner domain and the outer adapters; it holds
//! no business rules of its own and no protocol knowledge.

pub mod use_cases;
