//! The `generate` use case: turn a [`MarketConfig`] into a stored feed.
//!
//! Note what is *not* here: no path, no `create_dir_all`, no CSV. The use case
//! knows that a feed and its locate map are written together, and nothing else.
//! Where they land, and in what format, is the [`FeedStore`] adapter's business.
//!
//! It also does not print. The two lines the CLI shows bracket the call rather
//! than interleave with it, so the composition root can render them from the
//! config going in and the [`StoredFeed`] coming out — no output port needed.
//! (Contrast [`super::transmit`], where progress arrives *during* the operation
//! and an observer is the only way to surface it without inverting the layers.)

use crate::application::Result;
use crate::application::ports::{FeedStore, StoredFeed};
use crate::domain::market::{MarketConfig, MarketSimulator, symbol_table};
use crate::domain::message::ItchMessage;

/// Generates `config.count` messages and hands them to the store.
///
/// Materialised into a `Vec` rather than streamed. [`MarketSimulator`] is an
/// [`Iterator`], so a store that wanted to stream could take one directly and
/// never hold the feed in memory; at the slice-2 default of 100,000 messages
/// that buys nothing, and a slice is easier to hand to two writers.
pub fn generate_feed(store: &impl FeedStore, config: MarketConfig) -> Result<StoredFeed> {
    let messages: Vec<ItchMessage> = MarketSimulator::new(config).collect();
    Ok(store.save(&messages, &symbol_table())?)
}
