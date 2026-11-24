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

/// A script that spawns `join` gives it a socket as standard input (Node.js does); reading the link
/// must work as it does on a pipe, not fail by reopening /dev/stdin.
#[cfg(unix)]
#[test]
fn join_reads_its_link_from_a_socket() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let (mine, theirs) = UnixStream::pair().unwrap();
    let home = tempfile::tempdir().unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_one-mk")).arg("--home").arg(home.path()).args(["join", "--yes"])
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(theirs))).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    (&mine).write_all(b"not-a-link\n").unwrap();
    let output = child.wait_with_output().unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(error.contains("unsupported invitation") && !error.contains("No such device"), "{error}");
}
