//! Debug-build acceptance fixtures: only temporary workspaces and our own helper.
use crate::{
    team_mcp_broker::{Broker, Client, Pending},
    team_mcp_runtime::{LivePane, Runtime},
    team_mcp_stdio,
    teams_relay::TeamsRelayHub,
    workspace::{Store, Workspace},
};
use serde_json::{Value, json};
use std::{
    io::{BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

fn check(value: bool, message: &'static str) -> Result<(), String> {
    if value { Ok(()) } else { Err(message.into()) }
}

struct Helper {
    child: Child,
    stdin: Option<ChildStdin>,
    replies: Option<Receiver<Result<Value, String>>>,
}
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Helper {
    fn start(client: &Client, read_stdout: bool, missing_key: bool) -> Result<Self, String> {
        let mut command =
            Command::new(std::env::current_exe().map_err(|_| "Fixture executable unavailable")?);
        command
            .arg("--team-mcp-stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        client.apply_environment(&mut command);
        if missing_key {
            command.env_remove(crate::team_mcp_auth::ENV_KEY);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command
            .spawn()
            .map_err(|_| "Fixture helper could not start")?;
        let stdin = child.stdin.take();
        let replies = if read_stdout {
            let stdout = child.stdout.take().ok_or("Fixture stdout unavailable")?;
            let (sender, receiver) = mpsc::sync_channel(16);
            thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let result = match team_mcp_stdio::read_frame(&mut reader) {
                        Ok(Some(bytes)) => serde_json::from_slice::<Value>(&bytes)
                            .map_err(|_| "Non-JSON helper output".to_owned()),
                        Ok(None) => break,
                        Err(_) => Err("Malformed helper output".into()),
                    };
                    let failed = result.is_err();
                    if sender.send(result).is_err() || failed {
                        break;
                    }
                }
            });
            Some(receiver)
        } else {
            None
        };
        Ok(Self {
            child,
            stdin,
            replies,
        })
    }
    fn send(&mut self, value: Value) -> Result<(), String> {
        team_mcp_stdio::write_message(self.stdin.as_mut().ok_or("Fixture input closed")?, &value)
            .map_err(|_| "Fixture input write failed".into())
    }
    fn response(&self, mut poll: impl FnMut()) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let replies = self.replies.as_ref().ok_or("Fixture reader unavailable")?;
        loop {
            poll();
            match replies.recv_timeout(Duration::from_millis(5)) {
                Ok(reply) => return reply,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Fixture helper closed stdout".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    check(Instant::now() < deadline, "Fixture response timed out")?
                }
            }
        }
    }
    fn wait_exit(&mut self, timeout: Duration, success: bool) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().map_err(|_| "Fixture wait failed")? {
                return check(status.success() == success, "Unexpected helper exit status");
            }
            check(
                Instant::now() < deadline,
                "Helper did not exit within fixture deadline",
            )?;
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn initialize(&mut self) -> Result<(), String> {
        self.send(request(
            1,
            "initialize",
            json!({"protocolVersion":"2025-11-25",
            "capabilities":{},"clientInfo":{"name":"synthetic","version":"1"}}),
        ))?;
        let response = self.response(|| {})?;
        check(
            response["result"]["protocolVersion"] == "2025-11-25",
            "Helper initialize failed",
        )?;
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
    }
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}
fn pending(broker: &Broker) -> Result<Pending, String> {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(pending) = broker.try_next() {
            return Ok(pending);
        }
        check(
            Instant::now() < deadline,
            "Fixture request did not reach broker",
        )?;
        thread::sleep(Duration::from_millis(5));
    }
}

fn runtime_roundtrip() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|_| "Fixture directory unavailable")?;
    let (store, mut state) = Store::open(dir.path())?;
    let project = state.add_project(dir.path())?;
    let cwd = state.projects[0].path.clone();
    state.sessions.push(
        serde_json::from_value(json!({"id":7,"project":project,
        "title":"Synthetic", "launch":"ClaudeChat", "cwd":cwd}))
        .map_err(|_| "Invalid fixture")?,
    );
    store.save(&state)?;
    let pane = LivePane {
        pane: 7,
        project,
        cwd,
        local: true,
        active: true,
    };
    let mut runtime = Runtime::default();
    let mut hub = TeamsRelayHub::default();
    let launch = runtime.begin(&pane, &state)?;
    let mut helper = Helper::start(launch.client(), true, false)?;
    helper.initialize()?;
    helper.send(request(2, "tools/list", json!({})))?;
    let catalogue = helper.response(|| {
        runtime.poll(
            std::slice::from_ref(&pane),
            &mut state,
            Some(&store),
            &mut hub,
        );
    })?;
    let tools = catalogue["result"]["tools"]
        .as_array()
        .ok_or("Real helper catalogue has wrong shape")?;
    check(
        tools.iter().any(|tool| tool["name"] == "tessel_task_add"),
        "Missing task tool",
    )?;
    check(
        !tools.iter().any(|tool| tool["name"] == "tessel_browser"),
        "Unexpected browser authority",
    )?;
    helper.send(request(
        3,
        "tools/call",
        json!({"name":"tessel_task_add","arguments":{"title":"Synthetic MCP task"}}),
    ))?;
    let response = helper.response(|| {
        runtime.poll(
            std::slice::from_ref(&pane),
            &mut state,
            Some(&store),
            &mut hub,
        );
    })?;
    check(
        response["result"]["isError"] == false && state.tasks.len() == 1,
        "Real helper mutation failed",
    )?;
    let loaded: Workspace = serde_json::from_slice(
        &std::fs::read(dir.path().join("workspace.json"))
            .map_err(|_| "Fixture readback unavailable")?,
    )
    .map_err(|_| "Fixture persisted workspace invalid")?;
    check(
        loaded.tasks == state.tasks,
        "Real helper mutation was not persisted",
    )?;
    launch.revoke();
    helper.send(request(4, "tools/list", json!({})))?;
    let revoked = helper.response(|| {
        runtime.poll(
            std::slice::from_ref(&pane),
            &mut state,
            Some(&store),
            &mut hub,
        );
    })?;
    check(
        revoked.get("error").is_some(),
        "Revoked helper retained access",
    )?;
    drop(helper.stdin.take());
    helper.wait_exit(Duration::from_secs(3), true)
}

fn cancellation_and_missing_environment() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|_| "Fixture directory unavailable")?;
    let broker = Broker::bind()?;
    let launch = broker.launch(7, 9, dir.path())?;
    let mut helper = Helper::start(launch.client(), true, false)?;
    helper.initialize()?;
    helper.send(request(
        2,
        "tools/call",
        json!({"name":"synthetic","arguments":{}}),
    ))?;
    let held = pending(&broker)?;
    helper.send(
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while held.is_current() {
        check(
            Instant::now() < deadline,
            "Helper cancellation did not close broker exchange",
        )?;
        thread::sleep(Duration::from_millis(5));
    }
    check(
        held.finish(Ok(json!({"stale":true}))).is_err(),
        "Cancelled request completed",
    )?;
    helper.send(request(
        2,
        "tools/call",
        json!({"name":"synthetic","arguments":{}}),
    ))?;
    pending(&broker)?.finish(Ok(json!({"fresh":true})))?;
    let response = helper.response(|| {})?;
    let result: Value = serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .ok_or("Missing tool content")?,
    )
    .map_err(|_| "Invalid tool content")?;
    check(
        result == json!({"fresh":true}),
        "Cancelled request leaked a stale completion",
    )?;
    helper.send(request(9, "ping", json!({})))?;
    check(
        helper.response(|| {})?["id"] == 9,
        "Unexpected extra completion after cancellation",
    )?;
    drop(helper.stdin.take());
    helper.wait_exit(Duration::from_secs(3), true)?;

    let mut invalid = Helper::start(launch.client(), false, true)?;
    invalid.wait_exit(Duration::from_secs(3), false)?;
    let mut bytes = Vec::new();
    invalid
        .child
        .stdout
        .take()
        .ok_or("Fixture stdout unavailable")?
        .take(1024)
        .read_to_end(&mut bytes)
        .map_err(|_| "Fixture read failed")?;
    check(
        bytes.is_empty(),
        "Invalid helper configuration emitted stdout",
    )?;
    let mut error = String::new();
    invalid
        .child
        .stderr
        .take()
        .ok_or("Fixture stderr unavailable")?
        .take(1024)
        .read_to_string(&mut error)
        .map_err(|_| "Fixture error read failed")?;
    check(
        error.is_empty(),
        "Helper emitted unexpected diagnostic data",
    )
}

fn stalled_pipes_exit() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|_| "Fixture directory unavailable")?;
    let broker = Broker::bind()?;
    let launch = broker.launch(7, 9, dir.path())?;
    let mut partial = Helper::start(launch.client(), false, false)?;
    partial
        .stdin
        .as_mut()
        .ok_or("Fixture input missing")?
        .write_all(b"{")
        .map_err(|_| "Fixture write failed")?;
    let started = Instant::now();
    partial.wait_exit(Duration::from_secs(14), false)?;
    check(
        started.elapsed() >= Duration::from_secs(9),
        "Partial input failed before its deadline",
    )?;

    let mut unread = Helper::start(launch.client(), false, false)?;
    unread.send(request(1,"initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"synthetic","version":"1"}})))?;
    unread.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
    unread.send(request(2, "tools/list", json!({})))?;
    pending(&broker)?.finish(Ok(json!([{"name":"synthetic","description":"x".repeat(900_000),"inputSchema":{"type":"object"}}])))?;
    let started = Instant::now();
    unread.wait_exit(Duration::from_secs(14), false)?;
    check(
        started.elapsed() >= Duration::from_secs(8),
        "Output fixture did not exercise backpressure",
    )
}

pub fn run() -> Result<(), String> {
    runtime_roundtrip()?;
    cancellation_and_missing_environment()?;
    stalled_pipes_exit()?;
    println!("Team MCP process fixtures passed.");
    Ok(())
}
