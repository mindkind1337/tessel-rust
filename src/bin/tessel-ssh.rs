//! Console companion for an unwrapped Windows OpenSSH login. stdin and stdout
//! stay on ConPTY. Only client stderr passes through this process.
use std::{
    io::{self, Read, Write},
    process::{Command, Stdio},
};

const LOSS: i32 = 511;

#[derive(Default)]
struct Client {
    opened: bool,
    lost: bool,
}

impl Client {
    fn inspect(&mut self, line: &[u8], pid: u32) -> bool {
        let Ok(line) = std::str::from_utf8(line) else {
            return false;
        };
        let line = line.trim_end_matches(['\r', '\n']);
        // LogVerbose limits extra diagnostics to this function. The remote PTY
        // carries the shell's stderr on stdout, not through this parser.
        if line.starts_with("debug2: ")
            && line.contains("clientloop.c:client_status_confirm():")
            && let Some((_, message)) = line.split_once(&format!(" (pid={pid}): "))
        {
            if let Some(number) = message.strip_prefix("shell request accepted on channel ")
                && number.parse::<u32>().is_ok()
            {
                self.opened = true;
                self.lost = false;
            }
            return true;
        }
        self.lost |= line.starts_with("client_loop: send disconnect:")
            || line.starts_with("Connection reset by ")
            || (line.starts_with("Connection to ") && line.ends_with(" closed by remote host."))
            || (line.starts_with("Timeout, server ") && line.ends_with(" not responding."))
            || (line.starts_with("ssh: connect to host ")
                && [
                    "Connection refused",
                    "Connection timed out",
                    "Network is unreachable",
                ]
                .iter()
                .any(|reason| line.ends_with(reason)));
        false
    }
}

fn run() -> io::Result<i32> {
    let nonce = std::env::var("TESSEL_SSH_CHANNEL").unwrap_or_default();
    if nonce.len() != 36 || !nonce.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err(io::Error::other("Missing SSH session identity"));
    }
    let system = std::env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::other("Windows system directory unavailable"))?;
    let mut ssh = Command::new(std::path::PathBuf::from(system).join("System32/OpenSSH/ssh.exe"))
        .args([
            "-o",
            "IgnoreUnknown=LogVerbose",
            "-o",
            "LogVerbose=*clientloop.c:client_status_confirm():*",
        ])
        .args(std::env::args_os().skip(1))
        .env_remove("TESSEL_SSH_CHANNEL")
        .stderr(Stdio::piped())
        .spawn()?;
    let pid = ssh.id();
    let mut input = ssh.stderr.take().expect("piped SSH stderr");
    let mut output = io::BufWriter::new(io::stderr().lock());
    let mut state = Client::default();
    let mut buffer = [0u8; 4096];
    let mut line = Vec::new();
    let mut passthrough = false;
    let mut oversized = false;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        for &b in &buffer[..count] {
            if !oversized {
                line.push(b);
            }
            // Print interactive prompts immediately, even without a newline.
            if passthrough {
                output.write_all(&[b])?;
            } else if !(b"debug2: ".starts_with(&line) || line.starts_with(b"debug2: "))
                || line.len() >= 4096
            {
                output.write_all(&line)?;
                passthrough = true;
            }
            if line.len() >= 4096 {
                oversized = true;
            }
            if b == b'\n' {
                let opened = state.opened;
                let private = !oversized && state.inspect(&line, pid);
                if !passthrough && !private {
                    output.write_all(&line)?;
                }
                if state.opened && !opened {
                    write!(output, "\x1b]9;tessel-ssh-{nonce};R\x07")?;
                }
                line.clear();
                passthrough = false;
                oversized = false;
            }
        }
        output.flush()?;
    }
    if !passthrough {
        output.write_all(&line)?;
    }
    let code = ssh.wait()?.code().unwrap_or(1);
    Ok(if code == 255 && state.lost {
        LOSS
    } else {
        code
    })
}

fn main() {
    std::process::exit(run().unwrap_or_else(|err| {
        eprintln!("SSH: {err}");
        1
    }))
}

#[cfg(test)]
mod tests {
    use super::Client;

    #[test]
    fn shell_confirmation_is_specific_to_the_client_process() {
        let mut state = Client::default();
        let shell = b"debug2: clientloop.c:client_status_confirm():842 (pid=123): shell request accepted on channel 0";
        assert!(!state.inspect(shell, 321));
        assert!(!state.opened);
        assert!(state.inspect(b"debug2: clientloop.c:client_status_confirm():842 (pid=123): PTY allocation request accepted on channel 0",123));
        assert!(!state.opened);
        assert!(state.inspect(shell, 123));
        assert!(state.opened);
        state.inspect(b"Connection to local.test closed.\r\n", 123);
        assert!(!state.lost);
        state.inspect(b"client_loop: send disconnect: Unknown error\r\n", 123);
        assert!(state.lost);
    }
}
