#![cfg(debug_assertions)]
use std::{
    io::Read,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn original_executable_stdio_mode_preserves_protocol_scope_cancellation_and_pipe_deadlines() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tessel-rust"));
    command
        .arg("--self-test-team-mcp")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = Guard(
        command
            .spawn()
            .expect("start our own synthetic helper fixtures"),
    );
    let deadline = Instant::now() + Duration::from_secs(55);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "helper process fixtures timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut output = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .take(4096)
        .read_to_string(&mut output)
        .unwrap();
    let mut error = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .take(4096)
        .read_to_string(&mut error)
        .unwrap();
    assert!(
        status.success(),
        "synthetic process fixture failed: {error}"
    );
    assert_eq!(output.trim(), "Team MCP process fixtures passed.");
    assert!(error.is_empty());
}
