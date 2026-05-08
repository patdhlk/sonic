# sonic-executor

A Rust execution framework for [iceoryx2](https://github.com/eclipse-iceoryx/iceoryx2) —
inspired by Apex.Grace's `executor2` package.

## Crates

- `sonic-executor` — core: items, triggers, executor, runner, channels, services, chains, graphs, signal/slot.
- `sonic-executor-tracing` — `Observer` adapter that forwards lifecycle events to the global `tracing` subscriber.
- `sonic-replay` — placeholder reserved for future replay-coordinator integration; do not depend on it from production code.

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

## Publishing options

`Publisher<T>` exposes three send paths with different cost/ergonomics tradeoffs:

| Method | Sender-side cost | When |
|---|---|---|
| `send_copy(value)` | One move into shm | Tiny POD payloads (`u64`, small structs). Simplest. |
| `loan_send(\|t\| ... )` | `T::default()` + in-place mutation | Medium types where `Default` is cheap. |
| `loan(\|slot\| ... )` | None — closure constructs directly in shm | Large types or types without a sensible `Default`. |

For large types use `loan` with `MaybeUninit::write(value)` or iceoryx2's
`placement_default!` macro to get the full zero-copy benefit.

## Status

Pre-1.0. APIs may change. See `docs/superpowers/specs/` for the design notes (gitignored — request from a maintainer).

## Features

| Flag             | Default | Effect                                     |
|------------------|---------|--------------------------------------------|
| `tracing`        | off     | Add `Observer` integration target.         |
| `thread_attrs`   | off     | Core affinity + scheduling priority knobs. |
| `ctrlc-default`  | on      | SIGINT → `Stoppable::stop`.                |

## License

Apache-2.0 OR MIT.
