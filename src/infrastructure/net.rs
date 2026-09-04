//! The network adapters, and the one piece of addressing policy they share.
//!
//! [`resolve`] used to live at the crate root, where the transmit loop reached
//! *up* to `crate::resolve` to find it — an outward dependency from a mechanism
//! to the composition root, and the kind of thing a flat module list makes easy
//! to write and hard to see. It belongs next to the sockets that use it.

pub mod udp;

use std::io;
use std::net::{SocketAddr, ToSocketAddrs};

/// The port an operator gets when they name a host and nothing else.
pub const DEFAULT_PORT: &str = "9000";

/// Resolves a destination, defaulting the port when only a host is given.
pub fn resolve(dest: &str) -> io::Result<SocketAddr> {
    let owned;
    let with_port = if dest.contains(':') {
        dest
    } else {
        owned = format!("{dest}:{DEFAULT_PORT}");
        &owned
    };
    with_port.to_socket_addrs()?.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("{dest} resolved to no address"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_supplies_the_default_port() {
        let a = resolve("127.0.0.1").unwrap();
        assert_eq!(a.port(), 9000);
        let b = resolve("127.0.0.1:1234").unwrap();
        assert_eq!(b.port(), 1234);
        assert!(resolve("not a host name at all").is_err());
    }
}
