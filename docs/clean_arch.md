# Clean architecture layout

This document records the refactor that moved the crate from a flat `src/*.rs`
into layered directories, and why each module landed where it did. **No
implementation changed**: every function, type, signature and test body is
byte-identical to what it was before. The only edits were `use` paths, which the
compiler forces when a module moves.

## The rule being applied

Clean architecture has exactly one hard rule — *source code dependencies point
inward*. An inner ring may not name anything in an outer ring. Rust enforces
this weakly (any module can `use crate::anything`), so the layering is a
convention that this document exists to make auditable: the whole dependency
graph is the output of `grep -rn "use crate::" src/`, and it fits on one screen.

```
                       main.rs                (binary / composition root)
   ┌──────────────────────────────────────────────────────────┐
   │ infrastructure/   UDP socket, wall clock                 │  outermost
   │  ┌────────────────────────────────────────────────────┐  │
   │  │ application/    the four use cases                  │  │
   │  │  ┌──────────────────────────────────────────────┐  │  │
   │  │  │ adapters/     CSV, hex, session report        │  │  │
   │  │  │  ┌────────────────────────────────────────┐  │  │  │
   │  │  │  │ domain/   ITCH 5.0 + market simulation │  │  │  │  innermost
   │  │  │  └────────────────────────────────────────┘  │  │  │
   │  │  └──────────────────────────────────────────────┘  │  │
   │  └────────────────────────────────────────────────────┘  │
   └──────────────────────────────────────────────────────────┘
```

## Where each file went

| Before | After | Ring | Why |
| --- | --- | --- | --- |
| `src/model.rs` | `src/domain/model.rs` | domain | The ITCH entities. Plain `#[repr(C, packed)]` structs and field packing. No I/O, no allocation. |
| `src/codec.rs` | `src/domain/codec.rs` | domain | The ITCH 5.0 byte layout. See "Why the codec is domain" below. |
| `src/market.rs` | `src/domain/market.rs` | domain | The business rules being simulated: regimes, spreads, order lifecycle. Pure iterator, no clock. |
| `src/rng.rs` | `src/domain/rng.rs` | domain | Determinism-by-seed is a *rule* of this product, not a technical detail. See below. |
| `src/feed.rs` | `src/adapters/feed.rs` | adapters | Entity ⇄ CSV row translation, generic over `Write`/`BufRead`. |
| `src/formatter.rs` | `src/adapters/formatter.rs` | adapters | Entity ⇄ human string: hex dumps, scaled-integer prices, the banner. |
| `src/summary.rs` | `src/adapters/summary.rs` | adapters | A presenter: folds a `&[ItchMessage]` into a report. |
| `src/pacer.rs` | `src/infrastructure/pacer.rs` | infrastructure | Touches `Instant`, `sleep` and a spin loop — a driver for the wall clock. |
| `src/transmit.rs` | `src/infrastructure/transmit.rs` | infrastructure | Owns the `UdpSocket`. The only module that can lose a packet. |
| `src/lib.rs` (bodies) | `src/application/use_cases.rs` | application | `generate_signal`, `start_transmission`, `summarise`, `transmit_one`, `resolve` and the defaults they orchestrate with. |
| `src/lib.rs` | `src/lib.rs` | root | Reduced to crate docs, four `pub mod`s and the public façade. |
| `src/main.rs` | `src/main.rs` | binary | Unchanged except one import path. Cargo requires it at `src/main.rs`. |

## Decisions, and the community guidance behind them

### 1. Directory modules use `domain.rs`, not `domain/mod.rs`

Each layer is declared by a sibling file (`src/domain.rs`) next to its directory
(`src/domain/`), rather than by `src/domain/mod.rs`.

This is the layout the 2018 module-path changes ([RFC 2126]) introduced
specifically to avoid a tree full of identically-named `mod.rs` tabs, and it is
the form [the Rust Book, ch. 7.5][book-modules] presents first, with `mod.rs`
documented as "the older style … still supported". Clippy encodes both
conventions as mutually exclusive restriction lints —
[`clippy::mod_module_files`] and [`clippy::self_named_module_files`] — so the
community position is "pick one and be consistent"; this crate picks the newer
one.

The four layer files (`domain.rs`, `adapters.rs`, `application.rs`,
`infrastructure.rs`) and the new `lib.rs` contain **no logic** — only `pub mod`
declarations, the moved crate-level docs, and the re-export block. They are the
minimum the language requires to express a directory as a module; there is no
way to have a nested module tree in Rust without them.

### 2. `lib.rs` re-exports the use cases (façade)

```rust
pub use application::use_cases::{generate_signal, start_transmission, /* … */};
```

The [Rust API Guidelines, C-REEXPORT][c-reexport] state the rule directly: *"the
crate should re-export its most important types at the root, so that callers do
not have to know the internal module organisation."* Callers get
`realtime_stock_market_feed_transmitter::generate_signal` regardless of which
ring it lives in, so a later re-layering is not a breaking change. It also kept
`src/main.rs` almost untouched — only `formatter::dramatic_display` moved,
because it is deliberately *not* part of the façade.

### 3. Why the codec is in `domain/`, not `adapters/`

The textbook placement for a serializer is the adapter ring. Two things
overrule it here.

**It is the product.** This crate's reason to exist is emitting spec-correct
NASDAQ ITCH 5.0 bytes at line rate. The 36-byte Add Order layout is not an
interchange format chosen for convenience the way JSON-vs-protobuf would be; it
is the enterprise business rule. Uncle Bob's test — *would this rule exist if
the software didn't?* — passes: ITCH exists whether or not this program does.

**The compiler already said so.** Before the move, the dependency was mutual:
`codec` reads the structs in `model`, and `ItchMessage::wire_len` calls
`codec::wire_len`. Splitting them across a ring boundary would have manufactured
an outward dependency from the innermost ring — a genuine violation — that could
only be paid off by moving `wire_len`, i.e. by changing an implementation. A
dependency cycle between two modules is usually evidence that the boundary was
drawn in the wrong place, and that is what it was here.

### 4. Why `rng` is in `domain/`

A seeded xorshift generator looks like a utility. But `MarketSimulator` is
`Iterator`-pure precisely so that a `--seed` reproduces a feed byte for byte —
the property `csv_and_in_memory_generation_agree` asserts and the receiver
depends on. Reproducibility is a domain guarantee, and the generator that
provides it cannot sit outside the ring that promises it without an inward
dependency violation.

### 5. Why `feed` (CSV) is an adapter, but `transmit` and `pacer` are infrastructure

The distinguishing question is *does it name a device?*

`feed.rs` is generic — `write_feed<W: Write>`, `read_feed<R: BufRead>`. It knows
CSV; it does not know files. Its tests write to a `Vec<u8>`. That is exactly an
interface adapter: format translation with the device left as a parameter, and
in Rust the parameter is a `std::io` trait bound rather than an injected object
([Rust Book ch. 10][book-generics] — generics are monomorphised, so this
abstraction is free at run time, which matters at 10 kHz).

`transmit.rs` calls `UdpSocket::bind`. `pacer.rs` calls `Instant::now` and
`thread::sleep`. Both are non-substitutable OS drivers, and both are where the
program's real-world failure modes live. Outermost ring.

### 6. Naming

`domain` / `application` / `adapters` / `infrastructure` are lowercase
single-word module names per [C-CASE][c-case] and [RFC 430]. They are also the
names most widely recognised for these rings in Rust hexagonal-architecture
write-ups, which matters more than inventing crate-specific vocabulary: a
newcomer can guess where a file lives before opening the tree.

## Known remaining violations

These are real and deliberately **not** fixed, because every fix requires
changing an implementation, which this refactor was scoped out of.

1. **`application/use_cases.rs` depends on `infrastructure` and `adapters`
   directly** (`use crate::infrastructure::transmit::{…}`, `File::open`,
   `fs::create_dir_all`). This is the dependency rule broken in the classic
   place. The classic fix is ports and adapters: declare traits in
   `application` (`trait FeedSink`, `trait DatagramSender`), implement them in
   `infrastructure`, and have `main.rs` inject the implementations as the
   composition root. In Rust this is normally done with a generic parameter
   rather than `dyn Trait`, so it stays zero-cost. Until then, `application` is
   honestly a service layer, not an isolated interactor ring.

2. **`infrastructure/transmit.rs` reaches back to `crate::resolve` and
   `crate::DEFAULT_DEST`** (via the façade re-export). Outer-to-inner, so it does
   not break the dependency rule, but the defaults are configuration and belong
   in `TransmitConfig::default` rather than being read from another ring.

3. **`domain/codec.rs`'s test module uses `crate::adapters::formatter::hex`.**
   Inner ring reaching outward, confined to `#[cfg(test)]`. Harmless at run time
   — the code is not compiled into the binary — but it is the one place where a
   `grep` for outward dependencies gets a hit inside `domain/`.

4. **`application/use_cases.rs` still carries four tests for a removed `Args`
   type**, which is why `cargo test` does not compile. This predates the refactor
   (it arrived in `004ce96 clean up args`); the tests were moved unchanged along
   with everything else rather than silently deleted.

## What was explicitly not done

No file was split, merged, renamed or created beyond the module-declaration
files the language requires. No trait was introduced, no signature changed, no
`pub` narrowed to `pub(crate)`, no error type unified. Those are all defensible
next steps — item 1 above is the valuable one — but each changes behaviour or
API, and this pass was a pure move.

[RFC 2126]: https://rust-lang.github.io/rfcs/2126-path-clarity.html
[RFC 430]: https://rust-lang.github.io/rfcs/0430-finalizing-naming-conventions.html
[book-modules]: https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html
[book-generics]: https://doc.rust-lang.org/book/ch10-01-syntax.html
[c-reexport]: https://rust-lang.github.io/api-guidelines/necessities.html#c-reexport
[c-case]: https://rust-lang.github.io/api-guidelines/naming.html#c-case
[`clippy::mod_module_files`]: https://rust-lang.github.io/rust-clippy/master/index.html#mod_module_files
[`clippy::self_named_module_files`]: https://rust-lang.github.io/rust-clippy/master/index.html#self_named_module_files
