//! The daemon's log: operation names and numbers only, never a command, an input, a file's
//! content, an environment or a credential. At most about 2 MiB on disk.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 1024 * 1024;

pub struct Log {
    path: PathBuf,
    guard: Mutex<()>,
}

impl Log {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            guard: Mutex::new(()),
        }
    }

    pub fn event(&self, name: &'static str, fields: &[(&'static str, u64)]) {
        let Ok(_held) = self.guard.lock() else { return };
        if fs::metadata(&self.path)
            .map(|m| m.len() > MAX_BYTES)
            .unwrap_or(false)
        {
            let _ = fs::rename(&self.path, self.path.with_extension("log.1"));
        }
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut line = format!("{ms} {name}");
        for (key, value) in fields {
            line.push_str(&format!(" {key}={value}"));
        }
        line.push('\n');
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
}
