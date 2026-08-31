1. packet_seq: u64 — monotonic per datagram. Gaps mean lost datagrams. This is the only reason the receiver can distinguish "quiet market" from "network dropped 400 packets."
2. first_msg_seq: u64 + msg_count: u16 — a monotonic counter over messages, not packets. Packet sequence tells the receiver it lost 3 datagrams; message sequence tells it that cost exactly 137 messages. It also catches truncation: if the payload doesn't parse into exactly msg_count messages, the datagram is corrupt.
3. Heartbeats — zero-message packets that still burn a packet_seq. Without them, if the feed goes quiet and the last packets are lost, the receiver waits forever with no evidence anything is missing. A tail gap is invisible unless something keeps the sequence advancing.
4. An end-of-session flag — so the receiver can tell "stream ended cleanly" from "sender died." Send it a few times; it's UDP, it can be lost too.

Notice you don't need a payload_len field — recv_from hands the receiver the datagram length. Redundant length fields are a place for the two implementations to disagree.

One design consequence worth sitting with: packet_seq must increment per datagram sent, never per datagram the generator produced. If you ever retry a send_to that returned EWOULDBLOCK, resending must reuse the same sequence number, or you'll manufacture a phantom gap that the receiver reports as loss that never happened.

Build order I'd suggest

Each step is independently testable, which matters a lot when the failure mode is "the receiver on another machine sees garbage."

1. Codec — encode_*/decode_* against &[u8]. Test it round-trip, and hand-assert one known byte sequence so you catch a systematic endianness flip.
2. Framing — header + length-prefixed payload. Test: pack N random messages, unpack, get N back.
3. Generator — synthetic market data. The interesting constraint is self-consistency: you can't Delete an order you never Added. Keep live orders in a Vec and swap_remove on delete for O(1) random selection.
4. Pacer — the thing that makes it 100 Mbps and not "as fast as possible." A token bucket over bytes. thread::sleep alone has ~1 ms granularity on macOS, which is far too coarse at 8,900 pps; you'll want sleep-then-spin.
5. Socket — std::net::UdpSocket, plus SO_SNDBUF via unsafe extern "C" FFI to libc. That's not a crate dependency, so it's inside your rules. std doesn't expose setsockopt, and the default send buffer will bite you on bursts.
6. Metrics — atomics + a reporter thread. You need to see the achieved bitrate to know the pacer works.

Where do you want to start? I'd pick the codec — it's the piece your existing models.rs is already reaching toward, and getting the endianness discipline right early keeps it from contaminating everything above it. Tell me which one and I'll walk you through the design decisions without writing it for you.
