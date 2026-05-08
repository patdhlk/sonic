//! Two items connected by a `Channel<u64>`. The first publishes a counter
//! every 200ms; the second consumes it.

use core::time::Duration;
use iceoryx2::prelude::*;
use sonic_executor::{ControlFlow, Executor, item_with_triggers};

#[derive(Debug, Default, Clone, Copy, ZeroCopySend)]
#[repr(C)]
struct Count(u64);

/// Consumer item that owns the subscriber so `declare_triggers` and `execute`
/// both have access without any double-move issue.
struct Consumer {
    sub: sonic_executor::Subscriber<Count>,
}

impl sonic_executor::ExecutableItem for Consumer {
    fn declare_triggers(
        &mut self,
        d: &mut sonic_executor::TriggerDeclarer<'_>,
    ) -> Result<(), sonic_executor::ExecutorError> {
        d.subscriber(&self.sub);
        Ok(())
    }

    fn execute(&mut self, _ctx: &mut sonic_executor::Context<'_>) -> sonic_executor::ExecuteResult {
        while let Some(s) = self
            .sub
            .take()
            .map_err(|e| -> sonic_executor::ItemError { Box::new(e) })?
        {
            println!("got {}", s.payload().0);
        }
        Ok(ControlFlow::Continue)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut exec = Executor::builder().worker_threads(2).build()?;
    let ch = exec.channel::<Count>("sonic.examples.pipeline")?;
    let publisher = ch.publisher()?;
    let subscriber = ch.subscriber()?;

    // Producer item.
    let mut n = 0_u64;
    exec.add(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(200));
            Ok(())
        },
        move |_| {
            let _ = publisher
                .send_copy(Count(n))
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
            n += 1;
            Ok(ControlFlow::Continue)
        },
    ))?;

    // Consumer item reads every available message.
    exec.add(Consumer { sub: subscriber })?;

    exec.run()?;
    Ok(())
}
