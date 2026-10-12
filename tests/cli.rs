//! End-to-end tests for the `ccx` binary.

use std::process::Command;

#[test]
fn version_prints_ccx_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_ccx"))
        .arg("--version")
        .output()
        .expect("run ccx");
    assert!(out.status.success());
    let expected = format!("ccx {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}
