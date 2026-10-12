//! Smoke test for `testdata/fakebin/claude`.

use std::path::Path;
use std::process::Command;

#[test]
fn fake_claude_prints_argv_and_env_as_json() {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin/claude");
    let out = Command::new(&fake)
        .args(["-p", "two words", "--flag"])
        .env("CCX_TEST_VALUE", "a \"quoted\" value")
        .output()
        .expect("run fake claude");
    assert!(out.status.success());

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(
        json["argv"],
        serde_json::json!(["-p", "two words", "--flag"])
    );
    assert_eq!(json["env"]["CCX_TEST_VALUE"], "a \"quoted\" value");
    assert!(json["program"].as_str().unwrap().ends_with("claude"));
}
