use std::process::{Command, Stdio};

#[test]
fn noninteractive_join_requires_explicit_yes() {
    let output = Command::new(env!("CARGO_BIN_EXE_one-mk")).args(["join"]).stdin(Stdio::null()).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("non-interactive stdin requires --yes"));
}

#[test]
fn positional_link_warns_about_arguments_and_history() {
    let output = Command::new(env!("CARGO_BIN_EXE_one-mk")).args(["join", "--yes", "not-a-link"]).stdin(Stdio::null()).output().unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("process arguments") && error.contains("shell history"));
    assert!(error.contains("unsupported invitation"));
}
