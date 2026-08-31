## What "0% loss" actually asks of this half

The transmitter cannot guarantee zero loss — UDP has no retransmission, and a congested switch will drop your packets no matter how carefully you write this. What the transmitter can do is make loss detectable and exactly quantifiable downstream. That is the real deliverable of your header design.

Four things the receiver needs from you:

1. packet_seq: u64 — monotonic per datagram. Gaps mean lost datagrams. This is the only reason the receiver can distinguish "quiet market" from "network dropped 400 packets."
2. first_msg_seq: u64 + msg_count: u16 — a monotonic counter over messages, not packets. Packet sequence tells the receiver it lost 3 datagrams; message sequence tells it that cost exactly 137 messages. It also catches truncation: if the payload doesn't parse into exactly msg_count messages, the datagram is corrupt.
3. Heartbeats — zero-message packets that still burn a packet_seq. Without them, if the feed goes quiet and the last packets are lost, the receiver waits forever with no evidence anything is missing. A tail gap is invisible unless something keeps the sequence advancing.
4. An end-of-session flag — so the receiver can tell "stream ended cleanly" from "sender died." Send it a few times; it's UDP, it can be lost too.

-----

## Loss detection lives at a different layer than your messages

Look at what those four things describe. packet_seq, first_msg_seq, heartbeats, end-of-session — none of them are facts about a stock market. They're facts about a transmission. An ItchAddOrder doesn't know it was sent, or over what, or in what order relative to other datagrams. Putting a sequence number inside it would be modeling the pipe into the cargo.

So they go in a new type that wraps your messages:

┌─────────────────────────────────────────┐
│ envelope header  ← seq, count, session  │   ← new module, wire/
├─────────────────────────────────────────┤
│ [len][ITCH msg][len][ITCH msg][len]...  │   ← models.rs, untouched
└─────────────────────────────────────────┘

This is exactly why NASDAQ doesn't put sequence numbers in ITCH. They built a separate protocol, MoldUDP64, whose only job is to wrap a sequence of arbitrary messages in a datagram and make loss detectable. ITCH doesn't know MoldUDP64 exists. You can carry non-ITCH payloads over Mold, and you can carry ITCH over TCP without Mold. The layers are independent because the concerns are independent.

Keep that separation and your codec tests stay pure — you can round-trip an ItchAddOrder without constructing a fake packet, and you can test gap detection with dummy payloads without generating a single realistic order.

What MoldUDP64 actually looks like

Worth studying, because it does all four things in 20 bytes with no flag bits at all:

┌──────┬─────────────────┬─────────────────────────────────────┐
│ Size │      Field      │                                     │
├──────┼─────────────────┼─────────────────────────────────────┤
│ 10 B │ Session         │ identifies this transmitter run     │
├──────┼─────────────────┼─────────────────────────────────────┤
│ 8 B  │ Sequence Number │ of the first message in this packet │
├──────┼─────────────────┼─────────────────────────────────────┤
│ 2 B  │ Message Count   │                                     │
└──────┴─────────────────┴─────────────────────────────────────┘

Then the payload is [u16 length][message] repeated.

The elegant part is Message Count, which is overloaded three ways:

- 0 → heartbeat. No messages, sequence doesn't advance.
- 0xFFFF → end of session. Sentinel value, no flag byte needed.
- anything else → that many messages follow.

Two of your four requirements collapse into a field you needed anyway. That's what good protocol design looks like — and it's worth noticing before you reach for a flags: u8 out of habit.

The refinement: packet_seq isn't load-bearing

I listed packet_seq and first_msg_seq as separate requirements. Look closely at Mold and you'll see it carries only a message sequence. Here's why that's sufficient:

packet A:  first_msg_seq = 1000, count = 45   →  next expected = 1045
packet B:  first_msg_seq = 1045               →  ✓ nothing lost
packet B': first_msg_seq = 1090               →  ✗ lost exactly 45 messages

first_msg_seq + msg_count is the next expected sequence. The arithmetic gives you message-level loss for free, which is the number you actually care about — "we lost 45 order events," not "we lost a datagram."

So what would a separate packet_seq buy you? One thing: datagram-level loss counts, which is the metric you can cross-check against /proc/net/snmp and ethtool -S. When the receiver says "we lost 45 messages" and you want to know whether that was one bad datagram or forty, packet_seq answers it.

That's diagnostics, not correctness. Include it if you want the cross-check (I would — you're going to be debugging loss, and being able to reconcile your counters against the kernel's is worth 8 bytes). But now you're including it knowing it's a debugging affordance rather than a requirement, which is a better reason than "Claude listed it."

One consequence either way: heartbeats must not advance first_msg_seq. They carry no messages, so there's nothing to number. Which means a lost heartbeat is undetectable via message sequence — and that's fine, because a lost heartbeat costs you nothing. It only existed to prove the sequence was still where you left it.

The thing I under-specified: session identity

Mold spends half its header — 10 of 20 bytes — on a session field. I mentioned nothing about this, and it's a real gap. Here's the failure it prevents:

Your transmitter crashes at msg_seq = 8_000_000 and restarts. Sequence resets to 0. The receiver sees the counter jump backwards by eight million. What does it conclude?

Without a session field: it can't tell. Backwards jumps are ambiguous between "sender restarted" and "badly reordered old packet" and "someone is sending to the wrong port." The receiver either reports catastrophic phantom loss or silently resyncs and hides a real restart.

With one: sequence numbers are only comparable within a session. New session ID means "clean slate, this is a different run, start over." Unambiguous.

Mold uses 10 ASCII bytes. You don't need that — a u64 holding the transmitter's start time in nanos since epoch is self-describing, monotonic across restarts, and free to generate. That's your fifth field, and it's the one I'd least want to leave out.

The one real modeling change in the ITCH layer

Separate from loss detection, you're going to hit this immediately in step 2, so it's worth flagging now.

Right now you have seven unrelated structs. There's no type that means "an ITCH message." So you can't write:

fn encode(msg: &???, out: &mut [u8]) -> usize

...and you can't hold a heterogeneous run of them. You need a sum type:

pub enum ItchMessage {
    AddOrder(ItchAddOrder),
    AddOrderAttributed(ItchAddOrderAttributed),
    OrderExecuted(ItchOrderExecuted),
    // ...
}

Largest variant is 40 bytes, so the enum lands around 48 with discriminant and padding — irrelevant here.

A subtlety worth knowing: your transmitter doesn't strictly need this. The generator could write bytes straight into the packet buffer and skip the intermediate value entirely, which is faster. But your decoder needs somewhere to put a parsed message, and your round-trip tests need an owned value to compare. Write the enum. If profiling later says the copy matters, add a direct-to-buffer fast path alongside it — but you'll still want the enum for tests.

The length prefix is a real trade, not an obvious win

Mold puts [u16 length] before every message. But ITCH message length is fully determined by the type byte — A is always 36, D is always 19. A 7-entry lookup table gives you the length. So the prefix is redundant. Why spend it?

At ~30 bytes average, 2 bytes is 6.7% of your bandwidth — 6.7 Mbps of your 100. Not nothing.

What you buy: resynchronization. Without a length prefix, a receiver that encounters a message type it doesn't recognize has no idea how many bytes to skip. It can't find the next message. One unknown type desynchronizes the entire rest of the datagram, and every message after it is lost — silently, and without triggering your gap detection, because the sequence numbers were fine. That's the worst class of bug: data loss that your loss detector reports as healthy.

With the prefix, an unknown type costs you exactly one message. Skip length bytes, carry on.

For you specifically this matters more than usual, because the receiver is a separate codebase. The two implementations will drift. The day you add message type P and the receiver hasn't been redeployed, the prefix is what stands between "the receiver skips a few messages" and "the receiver silently corrupts everything after the first P."

Pay the 6.7%.

Concretely, then

src/models.rs — unchanged for loss detection. (Still worth the c_char → u8 and repr(packed) changes I mentioned, but those are codec concerns, not sequencing.)

New — an ItchMessage enum — so the codec and framer have a type to speak in.

New — src/wire/, a header type holding roughly:

┌────────────────────┬───────────────────────────────────────────────────────────────────────┐
│       Field        │                                Purpose                                │
├────────────────────┼───────────────────────────────────────────────────────────────────────┤
│ magic: u32         │ catch wrong-port traffic and garbage immediately                      │
├────────────────────┼───────────────────────────────────────────────────────────────────────┤
│ version: u8        │ you will change this format                                           │
├────────────────────┼───────────────────────────────────────────────────────────────────────┤
│ session_id: u64    │ restart detection — makes sequences comparable                        │
├────────────────────┼───────────────────────────────────────────────────────────────────────┤
│ first_msg_seq: u64 │ message-level gap detection (the load-bearing one)                    │
├────────────────────┼───────────────────────────────────────────────────────────────────────┤
│ msg_count: u16     │ 0 = heartbeat; sentinel = end of session                              │
├────────────────────┼───────────────────────────────────────────────────────────────────────┤
│ packet_seq: u64    │ optional — datagram-level counts to reconcile against kernel counters │
├────────────────────┼───────────────────────────────────────────────────────────────────────┤
│ send_ts_nanos: u64 │ optional — one-way latency measurement                                │
└────────────────────┴───────────────────────────────────────────────────────────────────────┘

Mold does its job in 20 bytes; that table is ~40, or 2.9% of a 1400-byte datagram. The extras aren't free but they're cheap, and magic plus version will save you real debugging time when two separately-written implementations meet for the first time.

---

Want to design the header layout properly next — field ordering, why you'd want it to be a fixed 40 bytes rather than "whatever the struct happens to be," and the encode/decode pair with the test that catches endianness flips? That's the natural next step and it folds the codec work in with it.

I also noticed you have docs/concerns.md open — if there's something in there you want me to react to, @ it and I'll take a look.