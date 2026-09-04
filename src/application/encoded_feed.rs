//! A feed encoded to bytes once, up front.
//!
//! Application-layer rather than transport-layer: *when* to pay the encoding
//! cost is a use-case decision, and the answer ("all of it before the clock
//! starts") is the same whether the bytes then go out over UDP, TCP, or into a
//! capture file. The transport receives datagrams, not messages.

use crate::domain::codec::{CodecError, MAX_MESSAGE_LEN, encode};
use crate::domain::message::ItchMessage;

/// Encodes every message up front, into one contiguous buffer.
///
/// Encoding inside the send loop would put a variable amount of work inside a
/// 100 µs budget, and the whole point of the pacer is that the only variable
/// thing in that budget is the syscall. This costs ~4 MB for 100,000 messages.
pub struct EncodedFeed {
    bytes: Vec<u8>,
    /// `(offset, length)` per message.
    index: Vec<(u32, u8)>,
}

impl EncodedFeed {
    pub fn encode_all(msgs: &[ItchMessage]) -> Result<Self, CodecError> {
        let mut bytes = Vec::with_capacity(msgs.len() * MAX_MESSAGE_LEN);
        let mut index = Vec::with_capacity(msgs.len());
        let mut scratch = [0u8; MAX_MESSAGE_LEN];
        for msg in msgs {
            let n = encode(msg, &mut scratch)?;
            index.push((bytes.len() as u32, n as u8));
            bytes.extend_from_slice(&scratch[..n]);
        }
        Ok(EncodedFeed { bytes, index })
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn total_bytes(&self) -> usize {
        self.bytes.len()
    }

    #[inline]
    pub fn datagram(&self, i: usize) -> &[u8] {
        let (off, len) = self.index[i];
        &self.bytes[off as usize..off as usize + len as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::codec::decode;
    use crate::domain::market::{MarketConfig, MarketSimulator};

    fn feed(count: u64) -> Vec<ItchMessage> {
        MarketSimulator::new(MarketConfig {
            count,
            ..Default::default()
        })
        .collect()
    }

    #[test]
    fn pre_encoding_preserves_every_datagram() {
        let msgs = feed(5_000);
        let encoded = EncodedFeed::encode_all(&msgs).unwrap();
        assert_eq!(encoded.len(), msgs.len());
        for (i, msg) in msgs.iter().enumerate() {
            let datagram = encoded.datagram(i);
            assert_eq!(
                datagram.len(),
                msg.wire_len(),
                "datagram {i} is the wrong length"
            );
            assert_eq!(
                &decode(datagram).unwrap(),
                msg,
                "datagram {i} decoded differently"
            );
        }
        let expected: usize = msgs.iter().map(|m| m.wire_len()).sum();
        assert_eq!(encoded.total_bytes(), expected);
    }

    #[test]
    fn an_empty_feed_encodes_to_nothing() {
        let encoded = EncodedFeed::encode_all(&[]).unwrap();
        assert!(encoded.is_empty());
        assert_eq!(encoded.total_bytes(), 0);
    }
}
