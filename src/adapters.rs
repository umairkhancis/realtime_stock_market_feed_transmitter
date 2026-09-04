//! Interface adapters: translation between the domain's in-memory entities and
//! the representations the outside world wants them in — CSV rows, hex dumps,
//! human-readable session reports.
//!
//! These modules are generic over `std::io::Read`/`Write` rather than naming a
//! `File`, so they convert without choosing a device. Choosing the device is
//! the outer layers' job.

pub mod feed;
pub mod formatter;
pub mod summary;
