//! Print "tick" once a second until Ctrl-C.

use core::time::Duration;
use sonic_executor::{ControlFlow, Executor, item_with_triggers};

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
