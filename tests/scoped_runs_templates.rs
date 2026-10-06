use std::process::Command;
#[test]
fn real_scoped_runs_and_template_cli_mcp_regressions() {
    let output = Command::new("node")
        .args([
            "--test",
            "tests/scoped_runs_templates.mjs",
            "tests/scoped_run_safety.mjs",
            "tests/scoped_publication.mjs",
        ])
        .env("WORK_TEST_BINARY", env!("CARGO_BIN_EXE_work"))
        .env("TMPDIR", std::env::temp_dir())
        .output()
        .expect("Node is required for bounded run/template public verification");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
