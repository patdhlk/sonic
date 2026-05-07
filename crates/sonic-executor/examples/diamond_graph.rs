//! Diamond DAG with the root triggered every 500ms.

use core::time::Duration;
use sonic_executor::{ControlFlow, Executor, item, item_with_triggers};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut exec = Executor::builder().worker_threads(4).build()?;

    let mut g = exec.add_graph();
    let root = g.vertex(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(500));
            Ok(())
        },
        |_| {
            println!("root");
            Ok(ControlFlow::Continue)
        },
    ));
    let left = g.vertex(item(|_| {
        println!("  left");
        Ok(ControlFlow::Continue)
    }));
    let right = g.vertex(item(|_| {
        println!("  right");
        Ok(ControlFlow::Continue)
    }));
    let merge = g.vertex(item(|_| {
        println!("merge");
        Ok(ControlFlow::Continue)
    }));
    g.edge(root, left);
    g.edge(root, right);
    g.edge(left, merge);
    g.edge(right, merge);
    g.root(root);
    g.build()?;

    exec.run()?;
    Ok(())
}
