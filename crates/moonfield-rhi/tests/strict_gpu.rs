use std::process::Command;

#[test]
fn test_skip_child() {
    if std::env::var_os("MOONFIELD_SKIP_TEST_CHILD").is_some() {
        moonfield_rhi::testing::skip_gpu("test device unavailable");
    }
}

#[test]
fn test_strict_gpu_rejects_skips() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "test_skip_child", "--nocapture"])
        .env("MOONFIELD_SKIP_TEST_CHILD", "1")
        .env("MOONFIELD_STRICT_GPU", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("strict GPU mode"));
}

#[test]
fn test_tolerant_gpu_allows_skips() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "test_skip_child", "--nocapture"])
        .env("MOONFIELD_SKIP_TEST_CHILD", "1")
        .env_remove("MOONFIELD_STRICT_GPU")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("skipping: test device unavailable"));
}
