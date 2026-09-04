# Clean architecture, in Rust

This document records how the transmitter was reorganised and, for each
decision, the Rust guidance behind it. It changes no behaviour: the generated
CSV is byte-identical, and `generate`, `transmit`, `summary`, `one`, `help` and
the unknown-command error all print exactly what they printed before.

## 1. The rule

Clean Architecture is one rule and a lot of consequences. The rule is that
**source dependencies point inward**. An inner ring may not name an outer one.
Everything else — ports, adapters, presenters — exists to make that rule
survivable when the inner ring nevertheless needs something from outside.

Four rings:

```text
              ┌──────────────────────────────────────────┐
              │            presentation                  │   cli, console,
              │  ┌────────────────────────────────────┐  │   summary, banner,
              │  │          application               │  │   format
              │  │  ┌──────────────────────────────┐  │  │
   infra──────┼──┼─▶│           domain             │◀─┼──┼── ports, use cases,
   csv, udp,  │  │  │  message, codec, market, rng │  │  │   reports
   pacer      │  │  └──────────────────────────────┘  │  │
              │  └────────────────────────────────────┘  │
              └──────────────────────────────────────────┘
```

`infrastructure` and `presentation` are siblings in the outer ring; neither may
name the other, and both may name the two inner ones.

## 2. What was actually wrong

The old layout was ten sibling modules under `src/`. Flat is not a sin — the
[Rust Book, ch. 7](https://doc.rust-lang.org/book/ch07-00-managing-growing-projects-with-packages-crates-and-modules.html)
is clear that you split modules when a file stops being coherent, not on a
schedule. But four specific arrows had gone the wrong way, and a flat list is
exactly the shape that hides them:

| Violation | Where | Why it matters |
|---|---|---|
| `transmit.rs` called `crate::resolve` | infrastructure → crate root | a mechanism reaching up into the composition root |
| `transmit.rs` called `crate::DEFAULT_DEST` | ditto | a deployment default baked into the send path |
| `codec.rs` tests called `crate::hex` / `crate::format_price` | domain → presentation | a wire-format rule asserting how a price *looks* |
| `lib.rs` held use cases *and* `File`/`PathBuf` plumbing | application ≡ infrastructure | `generate_signal()` could not run without a filesystem |

Plus two smaller ones: `feed.rs` (the CSV format) reached into
`MarketSimulator` to decide which symbol table to write, and `main.rs` mixed
argv handling with a 25-line usage constant and a dispatch table.

And the suite did not build. Commit `004ce96` ("clean up args") removed the
`Args` type; four tests in `lib.rs` still referenced it, so `cargo test` failed
to compile on `main`. Those tests are deleted — they exercise a type that no
longer exists — and the rest were rehomed with the code they cover.

## 3. Where everything went, and why

### `domain` — what an ITCH feed *is*

`message`, `codec`, `market`, `rng`. The test this ring is held to: *would this
code change if we swapped UDP for TCP, or CSV for Parquet?* If yes, it is not
domain.

**The codec is domain, not infrastructure — and this is the one placement worth
arguing.** Serialization is the textbook example of a detail, and Clean
Architecture would normally push it outward. Not here. ITCH's byte layout *is*
the product: the whole program exists to put those exact bytes on a wire. Run
the test both ways. Swap the transport, swap the archive format — `codec.rs`
does not move. Have NASDAQ revise the spec — `codec.rs` changes and so does
every use case built on it. That is the signature of an enterprise rule, not a
delivery mechanism. It is also what lets `ItchMessage::wire_len()` call
`codec::wire_len()` without inverting anything, which the alternative placement
would have forced. The *transport* and the *archive format* are the details,
and both are outside.

**The RNG is domain, and deliberately not a port.** The instinct is to invert
it: `trait RandomSource`, SplitMix64 as an adapter. That would be wrong here.
The domain does not require *randomness*, it requires *reproducibility* — "a
seed names a market" is the invariant the CSV ground truth rests on, and an
injectable generator is precisely a hole through which an adapter could break
it. Keeping it concrete also keeps it monomorphised on a path that runs
millions of times per feed. This is the
[C-GENERIC](https://rust-lang.github.io/api-guidelines/flexibility.html#functions-expose-intermediate-results-to-avoid-duplicate-work)
judgement call in reverse: genericity is cheap in Rust, but it is not free of
*meaning*, and here the meaning is wrong.

`market` split into three files — `symbols` (reference data), `config` (a plain
`Copy` value type), `simulator` (the only part that owns state). A 1,000-line
module where a caller who wants the ticker list has to pull in the generator is
a cohesion problem, not a line-count problem.

### `application` — what the program *does*

Use cases, expressed over traits. `generate::generate_feed`,
`transmit::transmit_feed`, `slice_one`, and the `ports` they depend on.

`generate_feed` is the clearest before/after. It used to open directories,
create files, flush writers and call `fs::metadata`. It now reads:

```rust
pub fn generate_feed(store: &impl FeedStore, config: MarketConfig) -> Result<StoredFeed> {
    let messages: Vec<ItchMessage> = MarketSimulator::new(config).collect();
    Ok(store.save(&messages, &symbol_table())?)
}
```

No path, no `File`, no CSV. That is dependency inversion doing its job: the use
case names `FeedStore`, never `CsvFeedStore`.

**There is no `summarise` use case, on purpose.** That command is a load
followed by a render. Wrapping `store.load()` in an application function that
adds nothing would be ceremony — the kind of thing that earns clean
architecture its reputation for cardboard layers. The composition root calls the
port directly, which is what a composition root is for. Knowing where *not* to
put a seam is as much a part of this as knowing where to put one.

### `infrastructure` — the adapters

`csv` (format + `CsvFeedStore`), `net` (`resolve` + the UDP adapters), `time`
(the pacer). Every one implements a port declared one ring in.

`csv` is split so the *format* can be tested without a disk: `serde` reads and
writes over any `BufRead`/`Write` and round-trips through a `Vec<u8>`; `store`
is the thin part that knows about `File` and `PathBuf`. That split follows the
[API guidelines' C-GENERIC](https://rust-lang.github.io/api-guidelines/flexibility.html)
advice to accept `impl Write` rather than a concrete `File`, and the payoff is
that eight of the nine CSV tests never create a file.

### `presentation` — the terminal, and the composition root

`cli` (usage, dispatch, operator defaults), `console` (the presenter),
`format` (pure rendering), `summary`, `banner`.

**`banner` earns its own file for one reason:** `figlet-rs` and `colored` are
the crate's only third-party dependencies, and that is the only file that names
them. Clean Architecture says frameworks belong in the outermost ring where they
can be deleted without consequence; here that is literally true, and
`tests/architecture.rs` asserts it. Given a brief that says *"implementation
should be non-standard crates dependency free"*, having the two crates that
violate it quarantined in one deletable file is not a stylistic win.

## 4. The ports, and how they dispatch

Four traits in `application/ports.rs`. Two Rust-specific decisions:

**Associated `Error` types, not `Box<dyn Error>`.** Each port carries
`type Error: std::error::Error + 'static`, following `FromStr`, `TryFrom` and
`Iterator`. The adapter keeps its own concrete error — `FeedError`,
`io::Error` — and the `'static + Error` bound is exactly what makes
`Box<dyn Error>: From<E>` apply, so a use case still writes `?`. Boxing at the
trait would have thrown away the adapter's error type at the boundary and
forced every implementor to allocate.

**Generics for collaborators, `dyn` for the presenter.**

```rust
pub fn transmit_feed(
    store: &impl FeedStore,
    transmitter: &mut impl FeedTransmitter,
    config: &TransmitConfig,
    observer: &mut dyn TransmitObserver,
) -> Result<TransmitReport>
```

`FeedStore` and `FeedTransmitter` are chosen once at the composition root, so
`impl Trait` monomorphises and the abstraction costs literally nothing at run
time — the standard answer to "won't the indirection be slow?", and the reason
ports are cheaper in Rust than in the languages the pattern came from.
`TransmitObserver` is `&mut dyn` because it is passed *through* the transmitter
into the send loop; making it generic would monomorphise the transport over the
presenter for no benefit, and it is called once per second, so one vtable hop is
beneath measurement. Static dispatch where it is hot and chosen at compile time;
dynamic dispatch where it is cold and passed along.

`TransmitObserver` is the *output port* — a presenter, in Clean Architecture's
vocabulary. The send loop pushes facts at it (`on_progress`, `on_start`) instead
of formatting strings, which is what let `println!` leave the transport
entirely. Every method has a default no-op body so a test implements it with an
empty `impl` block — the `SilentObserver` in `ports.rs` is three tokens.

### The one boundary drawn pragmatically

`FeedTransmitter::transmit` takes a whole `EncodedFeed`, not one datagram. The
more textbook split puts the send loop in the use case behind a per-datagram
port. It was not taken, and the reason is worth stating rather than hiding: the
loop shares a 100 µs deadline budget with the pacer clock, so a per-datagram
port would need a `Clock` port too, and the pacing mechanism and the syscall it
schedules would end up in different rings while sharing one hot budget. Keeping
the loop, the socket and the clock together in one adapter keeps them cohesive.
The property that actually matters — a use case that never names `std::net` —
is preserved either way.

`PaceStats` did move inward, from the pacer to `application::transmit`, because
it is part of what a run *reports*. `infrastructure` depending on `application`
points inward and is fine; the reverse would not have been. Its
`total_lateness_nanos` field stayed private and gained a `record()` method, so
the pacer folds in a measurement rather than poking at three fields across a
ring boundary ([C-STRUCT-PRIVATE](https://rust-lang.github.io/api-guidelines/predictability.html#c-struct-private)).

## 5. Rust-specific choices

**No `mod.rs`.** `src/domain.rs` + `src/domain/`, not `src/domain/mod.rs`. The
2018-and-later path style, and the one clippy's
[`self_named_module_files`](https://rust-lang.github.io/rust-clippy/master/#self_named_module_files)
lint prefers. Practically: eight files named `mod.rs` in your editor's tab bar
is its own argument.

**`application::Result<T = ()>`, not `Fallible`.** The old alias fixed `T` at
`()`, which is not a shape any std alias takes. The replacement is
module-qualified — `io::Result`, `fmt::Result`, `application::Result` — per the
[API guidelines' naming conventions](https://rust-lang.github.io/api-guidelines/naming.html),
with a default type parameter so the common spelling stays short while
`Result<StoredFeed>` remains available.

**Boxed errors in the application, concrete enums in the layers.** The
[Rust Book's I/O project](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html)
draws exactly this line, and it is kept: `CodecError` and `FeedError` stay
concrete enums with hand-written `Display`/`Error` impls (no `thiserror` — the
brief forbids the dependency), because a caller might match on them. The
application layer boxes, because its only consumer is `main`, which prints and
exits. The moment a second consumer appears — a receiver crate, an FFI boundary
— this becomes `enum AppError` with `#[non_exhaustive]`, and the ports'
associated `Error` types are already the seam that makes it a local change.

**A thin `main.rs`.** Twenty-two lines: banner, argv, exit code. Everything else
is in the library crate beside it, which is the split the Rust Book recommends
in the same chapter, for the same reason — a binary's `main` cannot be
integration-tested, so as little as possible should live there. `cli::run` takes
`&[String]` rather than reading the environment itself, which is what makes the
two CLI tests possible at all.

## 6. Enforcing it

**Rust modules do not enforce acyclicity.** `crate::domain` can name
`crate::presentation` and the compiler will agree. So directory layout alone is
a convention that documents intent and does nothing to preserve it — six months
of commits and it is a lie. There are exactly two ways to make the rule real:

1. **A workspace.** One crate per layer. Cargo *does* refuse a dependency cycle
   between crates, so the rule becomes a compile error. This is the right answer
   for a large codebase and the wrong one at 4,200 lines: you pay for it in
   build graph complexity, four `Cargo.toml`s, and `cargo test` no longer
   meaning "test everything".
2. **A test.** `tests/architecture.rs` reads every file under `src/`, strips
   comment lines (so a doc link pointing outward is prose, not a dependency),
   and asserts three things: no inner ring names an outer one; `colored` and
   `figlet_rs` are reachable from `presentation/banner.rs` and nowhere else; and
   the only top-level modules are the four rings, so a file dropped into `src/`
   cannot escape governance by belonging to no layer.

The second was chosen. It is ~150 lines, it runs in microseconds, and its
failure message names the file, the forbidden symbol, and the reason:

```text
the dependency rule is broken in 1 place(s):
  domain/rng.rs: names `crate::presentation` — the domain must not know a terminal exists
```

Migrate to the workspace when a second binary appears — the receiver in
`docs/session-layer.md` is the obvious candidate, and it will want `domain` and
nothing else.

## 7. What changed, precisely

Behaviour: nothing. Verified by regenerating `data/feed.csv` and diffing against
the pre-refactor file (byte-identical, 4,912,509 bytes), and by diffing the
stdout of `summary`, `help`, `one` and the unknown-command path against binaries
built from the previous commit.

Signatures that changed, all of them to fix a dependency arrow:

- `generate_signal()` / `start_transmission()` / `summarise()` / `transmit_one()`
  no longer exist as no-argument crate-root functions. They are use cases taking
  ports (`generate_feed`, `transmit_feed`, `slice_one`) plus CLI handlers that
  wire the adapters.
- `write_symbol_table(out)` → `write_symbol_table(out, entries)`. The CSV
  adapter no longer reaches into `MarketSimulator` to decide which universe to
  write; the use case passes the rows. That dependency was inward and therefore
  legal, but it let the *file format* make a domain choice.
- `MarketSimulator::symbol_table()` → `market::symbol_table()`. It never touched
  `self`; it is reference data, and it now lives with the reference data.
- `PaceStats` moved from `pacer` to `application::transmit` and gained
  `record()`.
- `Fallible` → `application::Result<T = ()>`.

Tests: 64 passing, up from a suite that did not compile. Four dead `Args` tests
deleted; the rest moved with their code; three cross-layer tests promoted to
`tests/feed_round_trip.rs` because no single module owns the property they
assert; three new tests in `tests/architecture.rs`; and small additions where a
rehoming left a gap (`format`'s price rendering, `cli`'s documented defaults,
`symbols`' locate numbering).

## 8. What was deliberately not done

- **`presentation::summary` still interleaves computation with `println!`.**
  Its pure half (`focus_mids`, `realized_vol`) is already separable and already
  tested. Splitting it into a `domain::analytics` that returns a struct and a
  presenter that renders it is the honest next step; it was skipped because it
  is a real implementation change, and this pass was a structural one.
- **`TransmitConfig.dest` is a `String` in the application layer.** Strictly, a
  destination is a transport detail. It stays because it is an opaque address
  the adapter interprets, and because moving it would change the config type
  for no gain until there is a second transport.
- **Nine clippy warnings survive** (`is_multiple_of`, byte-string literals,
  a collapsible `if`). All of them predate this work and all are in code that
  moved verbatim; fixing them would have meant changing implementations, which
  was out of scope.

## References

- [The Rust Programming Language, ch. 7 — Managing Growing Projects](https://doc.rust-lang.org/book/ch07-00-managing-growing-projects-with-packages-crates-and-modules.html)
- [The Rust Programming Language, ch. 12 — An I/O Project](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html) — the `main.rs` / `lib.rs` split, and boxed errors at the binary boundary
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) — C-GENERIC (accept `impl Write`), C-STRUCT-PRIVATE, C-COMMON-TRAITS, naming
- [Rust Error Handling — std, and the case for concrete library errors](https://doc.rust-lang.org/std/error/trait.Error.html)
- [clippy: `self_named_module_files`](https://rust-lang.github.io/rust-clippy/master/#self_named_module_files) — the no-`mod.rs` layout
- Robert C. Martin, *Clean Architecture* — the dependency rule, ports and adapters, presenters

----

## Explain each layer from outward to inward

Walking in from the edge. Note that the first two are siblings in the same ring, not stacked — neither may name the other.

---

main.rs — 22 lines above everything

dramatic_display("RT Transmitter");
let args: Vec<String> = env::args().skip(1).collect();
if let Err(e) = cli::run(&args) { eprintln!("..."); process::exit(1); }

Only the two things a binary can do that a library can't: read the process environment, set an exit code. Everything else is in the lib crate so it can be tested.

Ring 3a — presentation/

Human-facing. The only layer allowed to name a third-party crate.

- cli.rs — the composition root. USAGE, DEFAULT_CSV, and run(&[String]) dispatching the five commands. Each handler is the same shape: construct the concrete adapters, call one use case, render the result. This is the only file in the crate that knows a FeedStore is a CSV file.
- console.rs — ConsoleObserver, which implements TransmitObserver, plus print_report. This is where every println! from the old transmit path ended up.
- format.rs — hex(), format_price(). Pure, no I/O.
- summary.rs — the summary command's report: message mix, per-symbol table, per-second timeline, realized volatility.
- banner.rs — figlet-rs and colored, quarantined. Delete this file and both dependencies go with it.

Ring 3b — infrastructure/

Machine-facing. Every module here implements a port declared one ring in.

- csv/serde.rs — HEADER, FeedError, write_feed, read_feed, write_symbol_table. Generic over impl Write / impl BufRead, so its tests round-trip through a Vec<u8> and never create a file.
- csv/store.rs — CsvFeedStore, the FeedStore impl. All the File, PathBuf and create_dir_all in the program is in this one file.
- net.rs — DEFAULT_PORT and resolve(), which used to live at the crate root where the send loop reached up for it.
- net/udp.rs — UdpFeedTransmitter (the send loop, FeedTransmitter) and UdpDatagramSink (slice 1's one-shot, DatagramSink). The only file that touches UdpSocket.
- time/pacer.rs — Pacer: absolute deadlines from a fixed origin, sleep until 60 µs out then spin.

Ring 1 — application/

What the program does, with no mention of a socket, a file, or a terminal.

- ports.rs — the four traits (FeedStore, FeedTransmitter, DatagramSink, TransmitObserver) and their data types (StoredFeed, TransmitStart, TransmitProgress, SilentObserver). Each port carries an associated Error type so the adapter keeps its own concrete error.
- transmit.rs — TransmitConfig, PaceStats, TransmitReport, and the transmit_feed use case.
- generate.rs — generate_feed(&impl FeedStore, MarketConfig) -> Result<StoredFeed>. Two lines of body.
- slice_one.rs — the hand-built slice-1 message and its short-send check.
- encoded_feed.rs — EncodedFeed: one contiguous Vec<u8> plus an (offset, len) index, so nothing variable happens inside the 100 µs budget.

Ring 0 — domain/

What an ITCH feed is. No std::io, std::net, std::fs, std::thread, std::time, no println!, no dependencies.

- message.rs — the seven #[repr(C, packed)] records, the ItchMessage sum type, and the smart constructors (pack_itch_timestamp, pack_stock_symbol) that return Option rather than truncating.
- codec.rs — the wire format: the 11-byte shared prefix, the seven fixed lengths, encode/decode dispatch, CodecError.
- market/symbols.rs — SymbolSpec and the eight names; symbol_table().
- market/config.rs — MarketConfig and the session constants.
- market/simulator.rs — the generator: regime rolls, price steps, and the seven message builders that keep every reference live.
- rng.rs — SplitMix64. Concrete, not a port, because the domain needs reproducibility rather than randomness.

---

The whole thing, traced

main.rs                                  cli::run(["transmit"])
└─ presentation::cli::transmit()         builds CsvFeedStore, UdpFeedTransmitter,
   │                                     ConsoleObserver, TransmitConfig::default()
   └─ application::transmit_feed(store, transmitter, cfg, observer)
      ├─ store.load()          ──port──▶ CsvFeedStore ─▶ csv::serde::read_feed ─▶ domain::message
      ├─ observer.on_feed_loaded()  ──port──▶ ConsoleObserver ─▶ println!
      ├─ EncodedFeed::encode_all()  ─────────────────────────▶ domain::codec::encode
      ├─ observer.on_feed_encoded() ──port──▶ ConsoleObserver
      └─ transmitter.transmit() ──port──▶ UdpFeedTransmitter
                                          ├─ net::resolve, bind, connect
                                          ├─ observer.on_start()
                                          └─ loop: pacer.wait(i)
                                                   sock.send(feed.datagram(i))
                                                   observer.on_progress()
   └─ presentation::console::print_report(&report)

Every arrow crossing a ring boundary inward is a direct call. Every arrow crossing outward goes through a port. That's the whole design in one picture — transmit_feed is the only function that sees all four collaborators, and it names none of them concretely.

✻ Brewed for 53s · done 10:27 AM