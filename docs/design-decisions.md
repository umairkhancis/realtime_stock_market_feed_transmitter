## I am not understanding. Transmitter needs to generate stock market feed data continously. Where does the MTU, link rate comes into the picture and how IP fragmentation connects with this. teach me the design decisions involved here from fundamental principles step by step

Right — let me build this from the ground up. The missing link is that "continuous stream" and "UDP" are fundamentally incompatible ideas, and every constraint you've been asking about falls out of reconciling them.

Your repo has encode_add_order but no transmitter yet, so this is exactly the right time to work it out.

---

Step 0: The reframe

Your emitter produces a stream: an unbounded sequence of discrete events over time.

t →   [AddOrder] [AddOrder] [Cancel] [Trade] [AddOrder] ...

But UDP does not move streams. UDP moves datagrams — discrete, independently-addressed units, each delivered whole or not at all. There is no write() that just keeps flowing. Every send_to() call produces exactly one packet.

So the transmitter is forced to answer a question the stream itself never asks:

▎ Where do I cut?

That single decision — how many messages go into one datagram — is the design decision. MTU, line rate, fragmentation, and latency are
all just constraints that bound the answer. Let me tak

---

Step 1: The decision, and its two extremes

Option A: cut after every message
  send_to([AddOrder])  send_to([AddOrder])  send_to([Cancel]) ...
  → 1 message per packet

Option B: accumulate, cut when full
  send_to([AddOrder][AddOrder][Cancel][Trade]...×38)
  → 38 messages per packet

Option A is what src/lib.rs does today. It is the simpd it's the right call for slice 1.

Everything from here is about why Option B becomes mannd what bounds "how full is full."

---

Step 2: The upper bound — MTU

Every physical link has a Maximum Transmission Unit: tit will carry as one unit. I measured yours:

psx    enp0s1  mtu: 1500
trader enp0s1  mtu: 1500

1500 bytes is the Ethernet standard. Subtract the headers that must ride along:

1500  MTU
 -20  IP header
  -8  UDP header
─────
1472  bytes of UDP payload available

1472 is your hard ceiling per datagram. Not a guideline — the number past which the network layer must take action.

---

Step 3: What happens if you exceed it — IP fragmentation

Suppose you ignore the ceiling and hand the kernel a 3000-byte datagram. It doesn't fail. IP silently fragments it:

  sendto(3000 bytes)
         │
         ▼   IP splits into MTU-sized pieces
  ┌───────────────┬───────────────┬───────────────┐
  │ IP│UDP│1472 B │ IP│  1480 B   │ IP│    48 B   │
  │   frag 0      │   frag 1      │   frag 2      │
  └───────────────┴───────────────┴───────────────┘
         │               │               │
         ▼               ▼               ▼
    receiver's kernel holds fragments, reassembles,
    then delivers ONE 3000-byte datagram to recv_from()

It works. That's the trap — it works in testing and hurts in production. Four reasons it's wrong for market data:

1. Loss amplification. Reassembly is all-or-nothing. Lose any one fragment and the entire datagram is discarded. If per-frame loss is
p, a k-fragment datagram fails at roughly k × p. Splitples your loss rate — and each loss now costs you 3000bytes of feed instead of 1500.

2. No recovery. Under TCP, a lost fragment is retransmitted and you never notice. UDP has no retransmission. The data is gone
permanently.

3. Reassembly is stateful and bounded. The receiver hoa fixed-size buffer pool with a timeout. Under burstload that pool exhausts, and it drops fragments belonging to datagrams that would otherwise have completed — loss that spreads to
innocent packets.

4. Only fragment 0 carries the UDP header. Fragments 1rs. Stateless firewalls, ACLs, and load balancers can't classify them and frequently drop them outright.

Conclusion: cap the transmitter at 1472 bytes. Fragmentation is a failure mode to design away, not a feature to rely on.

Note this is a constraint on the sender. Your receiver's [0u8; 2048] is defensive sizing — it does not authorize sending 2048-byte
datagrams.

---

Step 4: The lower bound — latency

Why not always pack to 1472? Because filling a packet

To send 38 messages in one datagram, you must wait fort. That wait is pure added latency for message #1.

msg 1 arrives ──────────────────────────────► sent
                    (waiting for msgs 2..38)
              └────── added latency ─────────┘

How bad depends entirely on message rate:

┌───────────────┬───────────────────────┬─────────────
│ Message rate  │ Time to fill 38 slots │   Verdict    │
├───────────────┼───────────────────────┼─────────────
│ 347,000 msg/s │ 109 µs                │ invisible    │
├───────────────┼───────────────────────┼─────────────
│ 10,000 msg/s  │ 3.8 ms                │ acceptable   │
├───────────────┼───────────────────────┼─────────────
│ 1,000 msg/s   │ 38 ms                 │ unacceptable │
├───────────────┼───────────────────────┼─────────────
│ 100 msg/s     │ 380 ms                │ catastrophic │
└───────────────┴───────────────────────┴─────────────

So packing size can't be a constant. The resolution ismes first: the packet is full, or a timer expires.

loop {
    let msg = next_message();

    // would this message overflow the MTU budget? flush first.
    if buf.len() + 2 + msg.len() > MAX_PAYLOAD {   //
        flush();
    }
    buf.extend(msg.len().to_be_bytes());  // 2-byte length prefix
    buf.extend(msg);

    // don't let the oldest message sit too long.
    if first_msg_time.elapsed() > FLUSH_INTERVAL {   // e.g. 1ms
        flush();
    }
}

This gives you a bounded latency guarantee (never worsd full MTU efficiency whenever rate is high enough toearn it. Under load it self-tunes: packets fill naturally and the timer never fires. This is what real feed handlers do.

---

Step 5: The rate check — where line rate enters

Now you have a packet size. Line rate tells you whether your target message rate is achievable.

Convert your goal into a packet rate. "100 Mbps of ITCH data" at 36 bytes per message:

100 Mbps ÷ 8 = 12.5 MB/s ÷ 36 B = 347,222 messages/second

Now divide by messages-per-packet to get the packet rate each design demands, and compare against what your machines actually sustain
(measured earlier):

┌───────────────────────────┬──────────────────────┬────────┐
│          Design           │ Packets/sec required │ Measured ceiling │   Verdict    │
├───────────────────────────┼──────────────────────┼────────┤
│ Option A — 1 msg/packet   │ 347,222              │ ~150,000         │ impossible   │
├───────────────────────────┼──────────────────────┼────────┤
│ Option B — 38 msgs/packet │ 9,138                │ ~540,000         │ 59× headroom │
└───────────────────────────┴──────────────────────┴────────┘

This is the whole point. Same hardware, same 100 Mbps be done at any buffer size; framed it's not even closeto hard.

The reason is that cost is per-packet, not per-byte. Each packet burns:

- 66 bytes of wire overhead (preamble, Ethernet header, FCS, gap, IP, UDP) regardless of payload
- one sendto syscall on the transmitter, one recvfrom

Amortize that over 36 bytes and you waste 65% of the w4 bytes and you waste 4%.

---

Step 6: What batching forces you to add — deriving Mol

The moment you put N messages in one datagram, you've . Solve each and you will have re-derived MoldUDP64'sheader, field by field:

┌──────────────────────────────────────────────────────────────┬───────────────────────────┬─────────┐
│                   Problem batching created          olves it    │  Size   │
├──────────────────────────────────────────────────────────────┼───────────────────────────┼─────────┤
│ Receiver sees one blob — how many messages?                     │ 2 B     │
├──────────────────────────────────────────────────────────────┼───────────────────────────┼─────────┤
│ ITCH messages are variable-length — where does each ngth prefix │ 2 B/msg │
├──────────────────────────────────────────────────────────────┼───────────────────────────┼─────────┤
│ UDP drops packets. Losing one now loses 38 messages r           │ 8 B     │
├──────────────────────────────────────────────────────────────┼───────────────────────────┼─────────┤
│ Feed restarts — is seq 1 today's or yesterday's?                │ 10 B    │
└──────────────────────────────────────────────────────────────┴───────────────────────────┴─────────┘

That's the 20-byte header plus 2 bytes per block. MoldUDP64 isn't an arbitrary standard you're adopting — it's the minimal answer to
the problems batching creates. Working it out yourselfields.

The sequence number is the one that matters most. You P, so the design goal shifts from prevent to detect:

expected 1035, got 1041  →  6 messages lost, and you k

Without it, a dropped packet is indistinguishable from you can log the gap, request retransmission, or resync — and critically, you can measure your loss rate instead of guessing.

---

Step 7: The resulting design, with your numbers

Budget:      1500 MTU − 20 IP − 8 UDP        = 1472 bytes
Header:      MoldUDP64                       =   20 by
Per message: 2 length prefix + 36 Add Order  =   38 bytes
Capacity:    (1472 − 20) / 38                =   38 me
Packet:      20 + 38 × 38                    = 1464 bytes  ✓ under 1472

┌────────────────────────────────────────────────────────────┐
│ session(10) │ seq(8) │ count(2) │ blocks...
├─────────────┴────────┴──────────┼──────┬──────┬─────────────┤
│         20-byte header          │ len  │ msg  │ len
│                                 │ (2)  │ (36) │ (2) │(36) │ │
└─────────────────────────────────┴──────┴──────┴─────
                        1464 bytes total, ×38 messages

Transmitter rules:
1. Accumulate messages into a 1472-byte buffer.
2. Flush when adding the next message would exceed the budget.
3. Flush when the oldest message has waited > 1 ms.
4. Increment the sequence number by message count, not by 1.

Receiver rules:
1. buf ≥ 1472 → your existing 2048 is correct, unchang
2. Parse the header, then walk exactly count blocks.
3. If the blocks don't consume exactly n bytes, rejecttruncation check from src/lib.rs:27, upgraded from awarning to a hard error, because with a valid-looking header a truncated packet would otherwise decode as plausible garbage.
4. Track sequence gaps and count them.

---

One reality check on the 100 Mbps target

347,000 messages/second is roughly Nasdaq-scale. Actuarders of magnitude below that — realistically thousands of messages per day, not per second.

That's fine if 100 Mbps is a deliberate synthetic load target for learning the mechanics — it's a good one, because it puts you in the
regime where these decisions actually bite. But it chainates:

- At synthetic 100 Mbps: packing dominates. Packets alr fires.
- At real PSX rates: latency dominates. Packets almost never fill; the 1 ms timer fires constantly and you send mostly small packets —
  which is correct behavior, not a bug.

Building the flush-on-either-condition loop means the h regimes. That's why it's the design rather than afixed packing size.

---

The chain, in one line

Continuous stream → UDP forces you to cut it into packets → MTU caps the cut at 1472 bytes (exceed it and fragmentation multiplies
your loss) → latency stops you from always filling to yscall cost make packing mandatory anyway at high rates → batching creates framing problems → MoldUDP64 is the minimal header that solves them.



this stock market feed data model (layer 1) is a placeholder. I am interested in constant stream of data (layer 2) at max possible rate. Now teach me the design decisions.

Good — that's a completely different system from a market simulator, and now the earlier measurements become directly relevant. You're building a packet pump, and the design decisions are about finding and moving bottlenecks.

---

Step 1: "Max rate" of what? — the decision that reframes everything

There are two different maxima, and they differ by 25× on your setup:

┌───────────────────────────────────────┬─────────────────────────────────┐
│                                       │            Max rate             │
├───────────────────────────────────────┼─────────────────────────────────┤
│ Datagrams/sec (unframed, 1 msg each)  │ 1,043,470 msg/s                 │
├───────────────────────────────────────┼─────────────────────────────────┤
│ Messages/sec (packed 38 per datagram) │ 686,028 × 38 = 26,069,064 msg/s │
└───────────────────────────────────────┴─────────────────────────────────┘

Both were measured on your VMs. Same hardware, same 3-second blast. The only difference is how many messages ride in each sendto.

If "max possible rate" means messages, packing isn't an optimization — it's 25× and everything else is a rounding error. Decide this first, because it determines whether any of the remaining tuning is worth doing.

---

Step 2: The bottleneck moves with packet size

This is the key mechanic. Your two measurements bracket it:

size=36    →  1,043,470 pps  ×  102 B wire  =   851 Mbps   ← nowhere near link capacity
size=1464  →    686,028 pps  × 1530 B wire  =  8397 Mbps   ← at link capacity

At 36 bytes you're burning ~1 µs per packet in syscall + kernel stack overhead, and the data is irrelevant — 851 Mbps on a link that does ~7 Gbps. You are syscall-bound.

At 1464 bytes you're pushing ~1 GB/s and the packet rate has only dropped 34%. You've hit the virtio/memory data path. You are bandwidth-bound.

The crossover is where the two ceilings meet:

syscall ceiling    ≈ 1,000,000 pps
bandwidth ceiling  ≈ 875 MB/s  (7 Gbps)
crossover          ≈ 875 bytes/packet

messages/sec
     │                    ╭──────────── bandwidth-bound
 26M │                ╭───╯             (flat: bigger packets
     │            ╭───╯                  don't add messages)
     │        ╭───╯
     │    ╭───╯   ← syscall-bound
 1M  │╭───╯         (linear: every extra message
     ││              per packet is free)
     └┴────────────────────────────────
      36        875           1464  bytes

Design consequence: pack to the MTU. Below the crossover every additional message per packet is nearly free — you're paying for the syscall either way. Above it you'd be paying real bandwidth. 1464 bytes sits just past the knee, which is exactly where you want to be, and it's also the fragmentation limit. The three constraints agree.

---

Step 3: The bottleneck ladder

Once packed, here's where the remaining time goes and what to do about it, in payoff order:

┌─────┬───────────────────────────────────┬─────────────────────────────────────────────────────────┬───────────────────────┐
│  #  │            Bottleneck             │                           Fix                           │     Expected gain     │
├─────┼───────────────────────────────────┼─────────────────────────────────────────────────────────┼───────────────────────┤
│ 1   │ One syscall per message           │ Pack N messages per datagram                            │ 25× (measured)        │
├─────┼───────────────────────────────────┼─────────────────────────────────────────────────────────┼───────────────────────┤
│ 2   │ One syscall per datagram          │ sendmmsg / recvmmsg — batch 32–64 datagrams per syscall │ 2–5×                  │
├─────┼───────────────────────────────────┼─────────────────────────────────────────────────────────┼───────────────────────┤
│ 3   │ Kernel buffer too small           │ Raise SO_SNDBUF/SO_RCVBUF and net.core.rmem_max         │ eliminates burst loss │
├─────┼───────────────────────────────────┼─────────────────────────────────────────────────────────┼───────────────────────┤
│ 4   │ Allocation/formatting in hot loop │ Preallocate; counters not prints                        │ 10×+ on receiver      │
├─────┼───────────────────────────────────┼─────────────────────────────────────────────────────────┼───────────────────────┤
│ 5   │ Receive and decode on one thread  │ Ring buffer + worker thread                             │ ~2×                   │
├─────┼───────────────────────────────────┼─────────────────────────────────────────────────────────┼───────────────────────┤
│ 6   │ Single core                       │ SO_REUSEPORT + N threads/sockets                        │ ~N×                   │
├─────┼───────────────────────────────────┼─────────────────────────────────────────────────────────┼───────────────────────┤
│ 7   │ Kernel stack itself               │ io_uring, AF_XDP                                        │ 2–10× more            │
└─────┴───────────────────────────────────┴─────────────────────────────────────────────────────────┴───────────────────────┘
