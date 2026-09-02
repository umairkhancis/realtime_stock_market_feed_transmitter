## Slice 1

A small test where transmitter emits an ITCH message as a UDP packet and receiver on other machine with rust dependency free implementation and recover the same data.

## Slice 2

1:1 correspondence between a message and a UDP packet. 
Messages per second = Packets per second

On a 1 Gbps Ethernet Link:
1,000,000,000 / 36 = 27.7M packets per second. 

Due to 1:1 correspondence between message and packet, every 3600 microseconds, a new UDP packet is received. 

Switching between kernel space and user space 27.7M times per second is not feasible.

So, payload data rate is always lower than the link rate.

For feasibility reasons:
Transmitter emits data at 10,000 messages per second i.e. 10,000 packets per second at the uniform rate.
Transmitter emits messages that simulates a real market behavior.

Note: Generate 100,000 synthetic messages as CSV so that we can ask meanigful questions about the market behavior modulating in time at the receiver end.

### Implementation

What's new

- market.rs — the generator. Eight symbols, seeded, with a book per symbol. Every E/C/X/D/U names a reference a preceding A/F introduced and nothing has removed, so a receiver can replay it into a book and never hit a dangling reference. Behaviour modulates: opening burst, calm, a sharp mid-session shock, close ramp — plus per-symbol regime switching, with high-beta names taking more of the tape exactly when the market moves. Tests assert the shock is measurable, not just present.
- codec.rs — all seven message types, explicit big-ehrough a new ItchMessage enum. A and C are both 36bytes, so there's a test that length alone can't identify a message.
- feed.rs — lossless CSV. A round trip through text ytes; that's a test, since the CSV is only groundtruth if it's exact. The seq column is file bookkeeping and deliberately not on the wire.
- pacer.rs — absolute deadlines from a fixed origin not accumulated) with a sleep-then-spin hybrid.Lateness is measured and reported rather than assumed.
- transmit.rs, summary.rs, rng.rs (SplitMix64 — rand

model.rs's c_char fields became u8, which docs/sessid for; it removes a cast at every char field acrossseven encoders.

### Verify

```bash
cargo run -- gen                                    # Generates 100,000 messages -> `data/feed.csv` & `data/feed.symbols.csv`
cargo run -- send --csv data/feed.csv --dest HOST   # 1 message = 1 datagram, 10,000/s uniform
cargo run -- summary --csv data/feed.csv            # the answer key for the receiver
cargo run -- one HOST                               # slice 1, unchanged
```