//! Signal/slot pair triggered on an interval.

use core::time::Duration;
use sonic_executor::{item_with_triggers, signal_slot, ControlFlow, Executor};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut exec = Executor::builder().worker_threads(2).build()?;

    let (signal, slot) = signal_slot::pair::<u32>(&mut exec, "sonic.examples.signal_slot")?;

    let head = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(300));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    );
    let signal = signal.before_send(|n: &mut u32| {
        *n += 1;
        true
    });
    let slot = slot.after_recv(|n: &u32| {
        println!("slot received {n}");
        true
    });

    let chain: Vec<Box<dyn sonic_executor::ExecutableItem>> =
        vec![Box::new(head), Box::new(signal)];
    exec.add_chain(chain)?;
    exec.add(slot)?;

    exec.run()?;
    Ok(())
}
