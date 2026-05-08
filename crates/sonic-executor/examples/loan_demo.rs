#![allow(missing_docs, clippy::items_after_statements)]

use core::mem::MaybeUninit;
use core::time::Duration;
use iceoryx2::prelude::*;
use sonic_executor::{
    Channel, Context, ControlFlow, ExecutableItem, ExecuteResult, Executor, ExecutorError,
    ItemError, Subscriber, TriggerDeclarer,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, ZeroCopySend)]
#[repr(C)]
struct Big {
    tag: u64,
    payload: [u8; 1024],
}
// No Default impl needed — Channel<T> no longer requires it.
// `Publisher::loan` constructs the payload directly in shared memory.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut exec = Executor::builder().worker_threads(2).build()?;
    let topic = format!("sonic.demo.loan.{}", std::process::id());
    let ch: Arc<Channel<Big>> = Channel::open_or_create(exec.iceoryx_node(), &topic)?;
    let publisher = ch.publisher()?;
    let subscriber = ch.subscriber()?;

    let counter = Arc::new(AtomicU64::new(0));
    let c = Arc::clone(&counter);
    exec.add(sonic_executor::item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(100));
            Ok(())
        },
        move |_| {
            let n = c.fetch_add(1, Ordering::SeqCst);
            // Zero-copy send via loan: closure constructs Big directly in shm.
            let outcome = publisher
                .loan(|slot: &mut MaybeUninit<Big>| {
                    slot.write(Big {
                        tag: n,
                        payload: [0xAB; 1024],
                    });
                    true
                })
                .map_err(|e| -> ItemError { Box::new(e) })?;
            println!(
                "[producer] sent tag={n} via loan() (delivered to {} listeners)",
                outcome.listeners_notified
            );
            Ok(ControlFlow::Continue)
        },
    ))?;

    struct Consumer {
        sub: Subscriber<Big>,
    }
    impl ExecutableItem for Consumer {
        fn declare_triggers(&mut self, d: &mut TriggerDeclarer<'_>) -> Result<(), ExecutorError> {
            d.subscriber(&self.sub);
            Ok(())
        }
        fn execute(&mut self, _ctx: &mut Context<'_>) -> ExecuteResult {
            while let Some(s) = self.sub.take().map_err(|e| -> ItemError { Box::new(e) })? {
                let p = s.payload();
                println!(
                    "[consumer] received tag={} (payload[0]={:#x}, len={})",
                    p.tag,
                    p.payload[0],
                    p.payload.len()
                );
            }
            Ok(ControlFlow::Continue)
        }
    }
    exec.add(Consumer { sub: subscriber })?;

    exec.run_for(Duration::from_millis(500))?;
    Ok(())
}
