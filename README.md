# sonic-executor

A Rust execution framework on top of [iceoryx2](https://github.com/eclipse-iceoryx/iceoryx2) —
items triggered by IPC, intervals, and request/response activity; sequential chains;
parallel DAGs; signal/slot; lifecycle observability.

> [!WARNING]
> **Personal experiment. Not meant for production.**
> This crate exists to explore what a high-level execution framework on top of
> iceoryx2 looks like in Rust. The architecture is sound and the test suite is
> real, but the API has not stabilised, no version has been published, the
> `unsafe` story has not been independently audited, and there is no SLA,
> support, or backwards-compatibility guarantee. Use it to learn from, fork,
> or vendor in — not to ship.

## What's here

Three crates in the workspace:

- **`sonic-executor`** — core. Items, triggers, executor, runner, channels, services,
  chains, graphs, signal/slot, observer + execution monitor, optional thread tuning.
- **`sonic-executor-tracing`** — `Observer` adapter forwarding executor lifecycle
  and user events to the global `tracing` subscriber.
- **`sonic-replay`** — empty placeholder. Reserved for an eventual replay-coordinator
  integration; do not depend on it.

## Quick start

```rust,no_run
use core::time::Duration;
use sonic_executor::{item_with_triggers, ControlFlow, Executor};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut exec = Executor::builder().worker_threads(0).build()?;
    exec.add(item_with_triggers(
        |d| { d.interval(Duration::from_secs(1)); Ok(()) },
        |_| { println!("tick"); Ok(ControlFlow::Continue) },
    ))?;
    exec.run()?;
    Ok(())
}
```

Press Ctrl-C; the loop exits cleanly. iceoryx2 catches the signal at the `Node`
level; the WaitSet returns it; the executor honors it. No extra signal-handler
plumbing on your side.

## What an `ExecutableItem` looks like

The unit of work the executor schedules. `declare_triggers` registers what
should wake it (subscriber arrivals, intervals, deadlines, server requests,
client responses, raw listeners). `execute` runs once per wake-up and returns
either `Continue`, `StopChain`, or an error.

```rust
struct MyTask { /* state */ }

impl sonic_executor::ExecutableItem for MyTask {
    fn declare_triggers(
        &mut self,
        d: &mut sonic_executor::TriggerDeclarer<'_>,
    ) -> Result<(), sonic_executor::ExecutorError> {
        d.interval(core::time::Duration::from_millis(100));
        Ok(())
    }
    fn execute(
        &mut self,
        _ctx: &mut sonic_executor::Context<'_>,
    ) -> sonic_executor::ExecuteResult {
        // do work
        Ok(sonic_executor::ControlFlow::Continue)
    }
}
```

Or the closure-based path: `sonic_executor::item(closure)` and
`item_with_triggers(declare_closure, execute_closure)`.

## Publishing options

`Publisher<T>` exposes three send paths with different cost/ergonomics
tradeoffs. iceoryx2's zero-copy promise holds across the wire in every case
(the receiver always gets a reference to shared memory); the differences are
on the sender's side:

| Method | Sender-side cost | When to reach for it |
|---|---|---|
| `send_copy(value)` | One move into shm | Tiny POD payloads (`u64`, small structs). Simplest. |
| `loan_send(\|t\| ...)` | `T::default()` write + in-place mutation | Medium types where `Default` is cheap. |
| `loan(\|slot\| ...)` | None — closure constructs directly in shm | Large types or types without a sensible `Default`. |

For large types use `loan` with `MaybeUninit::write(value)` or iceoryx2's
`placement_default!` macro to get the full zero-copy benefit. The
[`loan_demo`](crates/sonic-executor/examples/loan_demo.rs) example sends 1 KB
payloads constructed entirely in shared memory.

## Composition

- `Executor::add(item)` — single item dispatched as one pool job.
- `Executor::add_chain([head, mid, tail])` — sequential walk; head's triggers
  gate the chain; `Ok(StopChain)` or `Err` from any item aborts the rest.
- `Executor::add_graph().vertex(...).edge(...).root(...).build()` — a DAG.
  Vertices run in parallel on the executor's thread pool when their predecessors
  complete. The root's triggers gate the graph.
- `wrap_with_condition(item, predicate)` — gate any item on a runtime check.
- `signal_slot::pair(&mut exec, topic)` — pre-built `ExecutableItem`s wrapping
  a `Channel<T>` for chain composition with `before_send`/`after_recv` hooks.

See `crates/sonic-executor/examples/` for runnable variants of each.

## Observability

- **`Observer`** trait — `on_executor_up/down/error`, `on_app_start/stop/error`,
  `on_send_event`. No-op default impls; non-blocking. The
  `sonic-executor-tracing` crate ships a ready-made adapter to the `tracing`
  ecosystem.
- **`ExecutionMonitor`** trait — `pre_execute(task, at)` /
  `post_execute(task, at, took, ok)`. Raw timestamps; build expectations on top.

Both are configured via `ExecutorBuilder::observer(...)` /
`ExecutorBuilder::monitor(...)`.

## Threading

Single executor-owned worker pool (M1 model). The thread that calls
`Executor::run()` becomes the WaitSet driver; pool workers run `execute()`.
For parallel graphs, use `worker_threads(N)` with `N >= 2`.

`Runner::new(exec, RunnerFlags::empty())` hosts the executor on a dedicated
OS thread; `Runner::stop()` joins it and re-throws any item error.

## Cargo features

| Flag             | Default | Effect                                     |
|------------------|---------|--------------------------------------------|
| `tracing`        | off     | Add the `tracing` crate as a dependency for adapter integrations. |
| `thread_attrs`   | off     | Core-affinity, thread name prefix, and (Linux) `SCHED_FIFO` priority on the executor's worker pool. |

iceoryx2 itself handles SIGINT/SIGTERM natively — no `ctrlc` feature is
needed and the loop exits cleanly on either signal.

## Detecting dropped notifications

When a publisher sends, iceoryx2 wakes each attached listener via a per-listener
Unix datagram socket. If the listener's kernel socket buffer is full, that
specific notification is dropped (the **data** still goes through the pub/sub
channel reliably; only the wakeup is lost). iceoryx2 logs a verbose
`FailedToDeliverSignal` warning when this happens.

**This usually means the consumer is falling behind.** Either it can't drain
fast enough, or the producer is bursting faster than the listener's socket
buffer can absorb. Lost wakeups can usually be tolerated (the listener will
still wake from a *previous* pending notification, drain everything, and
catch up), but if you have a deadline-sensitive consumer or zero-buffered
event semantics, every drop matters.

The publisher's send methods return [`NotifyOutcome`] so callers can detect
this programmatically without parsing logs:

```rust,no_run
# use sonic_executor::Publisher;
# fn run(publisher: Publisher<u64>) -> Result<(), Box<dyn std::error::Error>> {
let outcome = publisher.send_copy(42_u64)?;
if !outcome.delivered_to_any_listener() {
    // No listener received the wakeup. Either no subscribers are attached
    // (normal during startup), or every subscriber's socket was full.
    eprintln!("warn: send dropped, listeners_notified={}", outcome.listeners_notified);
}
# Ok(()) }
```

If you want to silence iceoryx2's verbose logging anyway (e.g. in production),
call `iceoryx2::prelude::set_log_level(LogLevel::Error)` once at startup. But
inspect `NotifyOutcome::listeners_notified` first — silencing the log without
checking the return is how dropped wakeups become silent bugs.

## Examples

```bash
cargo run --example interval_loop      # one tick per second; Ctrl-C to exit
cargo run --example pubsub_pipeline    # producer + consumer over a Channel<u64>
cargo run --example diamond_graph      # 4-vertex DAG fired by an interval
cargo run --example signal_slot        # signal/slot pair driven by a chain
cargo run --example loan_demo          # zero-copy 1 KB payloads via Publisher::loan
```

## Building

Workspace builds on stable Rust (edition 2024, MSRV 1.85). iceoryx2 0.8.x
is the underlying IPC layer.

```bash
cargo build --workspace
cargo test  --workspace --all-features -- --test-threads=1
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Tests must run single-threaded because each test creates its own iceoryx2
service in shared memory and parallel runs would contend on the same names.

## Status

This is **pre-1.0 personal experiment code.** Concretely:

- The API has not been audited by anyone other than the author.
- The `unsafe` blocks (cross-thread send of iceoryx2 ports, raw-pointer
  dispatch in the WaitSet callback) are documented but have not been
  reviewed by an `unsafe`-Rust expert or run under Miri.
- Several known polish items remain (see the design notes for the punch
  list); none are correctness-blocking, but the API surface should be
  considered unstable until they're addressed.
- iceoryx2 0.8.x is itself pre-1.0 and changes shape between versions;
  this crate is pinned to 0.8.1 and will need adaptation for later
  releases.
- No version has been published to crates.io. There is no support, no
  release cadence, no SLA, and no backwards-compatibility guarantee.

If any of those caveats matter for your use case, **don't ship it**.

Read the source, fork it, vendor it, or treat it as a worked example for
how to wire iceoryx2 into a higher-level execution framework — but don't
mistake it for a maintained library.

## License

Apache-2.0 OR MIT, at your option.
