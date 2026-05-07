//! Print "tick" once a second until Ctrl-C.

use core::time::Duration;
use sonic_executor::{item_with_triggers, ControlFlow, Executor};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut exec = Executor::builder().worker_threads(0).build()?;

    exec.add(item_with_triggers(
        |d| {
            d.interval(Duration::from_secs(1));
            Ok(())
        },
        |_| {
            println!("tick");
            Ok(ControlFlow::Continue)
        },
    ))?;

    exec.run()?;
    Ok(())
}
