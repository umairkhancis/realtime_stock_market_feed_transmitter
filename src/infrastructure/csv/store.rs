//! [`CsvFeedStore`] — the filesystem adapter behind
//! [`crate::application::ports::FeedStore`].
//!
//! Everything path-shaped in the program is concentrated here. The use cases
//! hold a `&impl FeedStore` and never learn that `data/feed.csv` exists.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::application::ports::{FeedStore, StoredFeed};
use crate::domain::message::ItchMessage;
use crate::infrastructure::csv::serde::{FeedError, read_feed, write_feed, write_symbol_table};

/// A feed CSV, plus the locate map written beside it.
#[derive(Debug, Clone)]
pub struct CsvFeedStore {
    path: PathBuf,
}

impl CsvFeedStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        CsvFeedStore { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `data/feed.csv` -> `data/feed.symbols.csv`.
    pub fn symbols_path(&self) -> PathBuf {
        symbols_path(&self.path)
    }
}

/// The locate map sits beside the feed rather than inside it: the two have
/// different lifetimes, and a receiver may keep one map across many feeds.
fn symbols_path(feed: &Path) -> PathBuf {
    let stem = feed
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    feed.with_file_name(format!("{stem}.symbols.csv"))
}

impl FeedStore for CsvFeedStore {
    type Error = FeedError;

    fn location(&self) -> String {
        self.path.display().to_string()
    }

    fn load(&self) -> Result<Vec<ItchMessage>, FeedError> {
        read_feed(BufReader::new(File::open(&self.path)?))
    }

    fn save(
        &self,
        messages: &[ItchMessage],
        symbols: &[(u16, &'static str, u32)],
    ) -> Result<StoredFeed, FeedError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let mut feed = BufWriter::new(File::create(&self.path)?);
        let rows = write_feed(&mut feed, messages.iter().copied())?;
        feed.flush()?;

        let symbols_path = self.symbols_path();
        let mut table = BufWriter::new(File::create(&symbols_path)?);
        write_symbol_table(&mut table, symbols)?;
        table.flush()?;

        Ok(StoredFeed {
            rows,
            feed_bytes: fs::metadata(&self.path)?.len(),
            feed_location: self.path.display().to_string(),
            symbols_location: symbols_path.display().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_path_sits_beside_the_feed() {
        assert_eq!(
            symbols_path(Path::new("data/feed.csv")),
            Path::new("data/feed.symbols.csv")
        );
        assert_eq!(
            symbols_path(Path::new("run1.csv")),
            Path::new("run1.symbols.csv")
        );
    }
}
