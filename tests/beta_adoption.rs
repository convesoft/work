//! Connected public CLI/MCP acceptance; the same harness runs on npm installs.
use std::process::Command;

#[test]
fn complete_beta_workflow_and_public_surface_parity() {
    let output = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/verify-beta.mjs"
        ))
        .arg(env!("CARGO_BIN_EXE_work"))
        .output()
        .expect("beta acceptance requires Node.js >=18, as packaged smoke does");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
