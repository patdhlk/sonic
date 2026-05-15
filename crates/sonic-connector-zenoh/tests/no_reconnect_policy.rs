//! `TEST_0309` — anti-req: verify the `sonic-connector-zenoh` public
//! API does NOT expose any `ReconnectPolicy` type or
//! `reconnect_policy` field. Verifies `REQ_0441`.
//!
//! Uses `cargo public-api` as a subprocess to dump the crate's
//! public surface and asserts no matching identifiers appear.
//! Requires `cargo-public-api` to be installed; install via
//! `cargo install cargo-public-api --locked` (CI installs it as
//! a setup step).

use std::process::Command;

#[test]
#[ignore = "requires cargo-public-api; run from ci-zenoh.yml public-api-tests job"]
fn public_api_does_not_mention_reconnect_policy() {
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
    let needles = ["ReconnectPolicy", "reconnect_policy"];
    for needle in needles {
        let hits: Vec<&str> =
            api.lines().filter(|l| l.contains(needle)).collect();
        assert!(
            hits.is_empty(),
            "REQ_0441 violated: public API mentions '{needle}':\n{}",
            hits.join("\n")
        );
    }
}

fn workspace_root() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR points to the crate dir; the workspace
    // root is two levels up (crates/sonic-connector-zenoh -> ..).
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate parent")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}
