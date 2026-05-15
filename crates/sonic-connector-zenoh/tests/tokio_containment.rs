//! `TEST_0314` (static piece) — verify the `sonic-connector-zenoh`
//! public API does NOT name any `tokio::` type in its surface.
//! Verifies the static half of `REQ_0403`.
//!
//! The runtime piece — asserting no tokio task handle attributable
//! to the zenoh sidecar appears in the executor's task list — is
//! deferred to a future stage that lands the necessary executor
//! introspection. See `TODO(Z6)` below.

use std::process::Command;

#[test]
fn public_api_does_not_name_tokio_types() {
    let output = Command::new("cargo")
        .args([
            "public-api",
            "--manifest-path",
            "crates/sonic-connector-zenoh/Cargo.toml",
            "--simplified",
        ])
        .current_dir(workspace_root())
        .output()
        .expect(
            "cargo public-api invocation failed; install with \
             `cargo install cargo-public-api --locked`",
        );
    assert!(
        output.status.success(),
        "cargo public-api exited non-zero:\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let api = String::from_utf8_lossy(&output.stdout);
    let leaks: Vec<&str> =
        api.lines().filter(|l| l.contains("tokio::")).collect();
    assert!(
        leaks.is_empty(),
        "REQ_0403 violated: public API names tokio types:\n{}",
        leaks.join("\n")
    );
}

// TODO(Z6): runtime piece — after `sonic-executor` exposes a public
// way to enumerate registered tasks/items, add a test that
// instantiates a `ZenohConnector` with `MockZenohSession`, registers
// it with an `Executor`, and asserts no tokio-attributable task
// handle is enumerable from outside the crate. See spec
// `TEST_0314` second paragraph.

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate parent")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}
