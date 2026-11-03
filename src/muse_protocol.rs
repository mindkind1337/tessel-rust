// Adapted from Tessel main/museServe.js / Muse session protocol.
//! A bounded model-list exchange only: no session, prompt, or user-input request.
use crate::{
    agent_model_probe::Failure,
    agent_model_process::{Error, Units},
};
use serde_json::{Value, json};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

const MAX_OUTPUT: usize = 1024 * 1024;

async fn send(pipe: &mut tokio::process::ChildStdin, value: Value) -> Result<(), Failure> {
    let mut bytes = serde_json::to_vec(&value).map_err(|_| Failure::Process(Error::Io))?;
    bytes.push(b'\n');
    pipe.write_all(&bytes)
        .await
        .map_err(|_| Failure::Process(Error::Io))
}

/// Worker-only. Caller resolves the native executable and supplies its fixed
/// `serve` argument. Stderr is drained without retaining or logging its contents.
pub fn model_list(mut command: Command, timeout: Duration) -> Result<Value, Failure> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Failure::Process(Error::Start))?;
    runtime.block_on(async {
        command.kill_on_drop(true).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn().map_err(|_| Failure::Process(Error::Start))?;
        let mut input = child.stdin.take().ok_or(Failure::Process(Error::Io))?;
        let mut output = child.stdout.take().ok_or(Failure::Process(Error::Io))?;
        let mut errors = child.stderr.take().ok_or(Failure::Process(Error::Io))?;
        let drain = tokio::spawn(async move { let _ = tokio::io::copy(&mut errors, &mut tokio::io::sink()).await; });
        let result = tokio::time::timeout(timeout, async {
            send(&mut input, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"tessel","title":"Tessel","version":env!("CARGO_PKG_VERSION")},"capabilities":{"userInputDialogs":false}}})).await?;
            let mut initialized = false;
            let mut pending = Vec::new();
            let mut units = Units::default();
            let mut total = 0;
            let mut buffer = [0u8;8192];
            loop {
                let n = output.read(&mut buffer).await.map_err(|_| Failure::Process(Error::Io))?;
                total += units.push(&buffer[..n], n == 0);
                if total > MAX_OUTPUT { return Err(Failure::Process(Error::TooMuch)); }
                if n == 0 { return Err(Failure::Failed(String::new())); }
                pending.extend_from_slice(&buffer[..n]);
                let mut consumed = 0;
                while let Some(offset) = pending[consumed..].iter().position(|b| *b == b'\n') {
                    let end = consumed + offset;
                    let line = String::from_utf8_lossy(&pending[consumed..end]);
                    consumed = end + 1;
                    let Ok(message) = serde_json::from_str::<Value>(crate::agent_env::trim(&line)) else { continue; };
                    if message.get("method").is_some() { continue; }
                    if !initialized && message["id"] == 1 {
                        if !message["result"].is_object() && !message["result"].is_array() {
                            return Err(Failure::Failed(String::new()));
                        }
                        initialized = true;
                        send(&mut input, json!({"jsonrpc":"2.0","method":"initialized"})).await?;
                        send(&mut input, json!({"jsonrpc":"2.0","id":2,"method":"model/list"})).await?;
                    } else if initialized && message["id"] == 2 {
                        if message["error"].is_object() {
                            // Bounded detail is returned to the picker, never logged or cached.
                            let code = message["error"]["code"].as_i64().unwrap_or(0);
                            let detail = message["error"]["message"].as_str().unwrap_or("");
                            let detail = String::from_utf16_lossy(&detail.encode_utf16().take(200).collect::<Vec<_>>());
                            return Err(Failure::Failed(format!("{code}: {detail}")));
                        }
                        return Ok(message["result"].clone());
                    }
                }
                if consumed > 0 { pending.drain(..consumed); }
            }
        }).await.unwrap_or(Err(Failure::Process(Error::Timeout)));
        // EOF on stdin is Muse's normal exit request. Give it the source grace
        // period, then kill/reap the direct child before releasing the worker.
        drop(input);
        if !matches!(tokio::time::timeout(Duration::from_secs(2), child.wait()).await, Ok(Ok(_))) {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
        }
        drain.abort();
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "muse_protocol::tests::child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("TESSEL_MUSE_FIXTURE", mode);
        command
    }
    #[test]
    fn exchange_sends_only_handshake_and_model_query_and_closes_input() {
        let value = model_list(fixture("success"), Duration::from_secs(3)).unwrap();
        assert_eq!(value["models"][0]["modelId"], "synthetic");
    }
    #[test]
    fn malformed_handshake_server_error_budget_and_timeout_fail() {
        for mode in ["bad-init", "exit"] {
            assert!(
                matches!(
                    model_list(fixture(mode), Duration::from_secs(3)),
                    Err(Failure::Failed(_))
                ),
                "{mode}"
            );
        }
        assert_eq!(
            model_list(fixture("server-error"), Duration::from_secs(3)),
            Err(Failure::Failed("-1: Synthetic failure".into()))
        );
        assert_eq!(
            model_list(fixture("flood"), Duration::from_secs(3)),
            Err(Failure::Process(Error::TooMuch))
        );
        assert_eq!(
            model_list(fixture("timeout"), Duration::from_millis(100)),
            Err(Failure::Process(Error::Timeout))
        );
    }
    #[test]
    #[ignore = "Synthetic protocol child invoked by parent tests"]
    fn child_fixture() {
        let mode = std::env::var("TESSEL_MUSE_FIXTURE").unwrap();
        let mut input = std::io::stdin().lock().lines();
        let first: Value = serde_json::from_str(&input.next().unwrap().unwrap()).unwrap();
        assert_eq!(first["method"], "initialize");
        assert_eq!(first["params"]["capabilities"]["userInputDialogs"], false);
        if mode == "exit" {
            return;
        }
        if mode == "timeout" {
            std::thread::sleep(Duration::from_secs(10));
            return;
        }
        if mode == "flood" {
            print!("{}", "x".repeat(MAX_OUTPUT + 1));
            std::io::stdout().flush().unwrap();
            return;
        }
        if mode == "bad-init" {
            println!("{{\"id\":1,\"error\":{{}}}}");
            return;
        }
        // Malformed text, notifications and server requests cannot trigger sessions.
        println!(
            "not json\n{{\"method\":\"notification\"}}\n{{\"id\":1,\"method\":\"ask\"}}\n{{\"id\":1,\"result\":{{}}}}"
        );
        std::io::stdout().flush().unwrap();
        let notified: Value = serde_json::from_str(&input.next().unwrap().unwrap()).unwrap();
        assert_eq!(notified["method"], "initialized");
        assert!(notified.get("id").is_none());
        let query: Value = serde_json::from_str(&input.next().unwrap().unwrap()).unwrap();
        assert_eq!(query, json!({"jsonrpc":"2.0","id":2,"method":"model/list"}));
        if mode == "server-error" {
            println!("{{\"id\":2,\"error\":{{\"code\":-1,\"message\":\"Synthetic failure\"}}}}");
        } else {
            println!("{{\"id\":2,\"result\":{{\"models\":[{{\"modelId\":\"synthetic\"}}]}}}}");
        }
        std::io::stdout().flush().unwrap();
        assert!(input.next().is_none(), "No further requests are permitted");
    }
}
