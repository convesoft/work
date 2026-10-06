use std::process::Command;
#[test]
fn real_hosting_cli_and_mcp_regressions() {
    let output = Command::new("node")
        .args(["--test", "tests/scope_hosting.mjs"])
        .env("WORK_TEST_BINARY", env!("CARGO_BIN_EXE_work"))
        .env("TMPDIR", std::env::temp_dir())
        .output()
        .expect("Node is required for public hosting verification");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
