//! Parse the current draft in an isolated process. No project path or user
//! environment is needed to compute a syntax tree.
use crate::outline_tree::{self, Symbol};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_OUTPUT: usize = 16 * 1024 * 1024;
static SLOT: Mutex<()> = Mutex::new(());
type ResultTree = Result<Arc<Vec<Symbol>>, ()>;

pub(crate) struct Pending {
    cancel: Arc<AtomicBool>,
    ready: Arc<OnceLock<ResultTree>>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl Pending {
    pub(crate) fn start(text: Arc<str>, ts: bool) -> Arc<Self> {
        let cancel = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(OnceLock::new());
        let pending = Arc::new(Self {
            cancel: cancel.clone(),
            ready: ready.clone(),
        });
        let launched = std::thread::Builder::new()
            .name("script outline".into())
            .spawn(move || {
                let start = Instant::now();
                while !cancel.load(Ordering::Acquire) {
                    if start.elapsed() >= Duration::from_millis(180)
                        && let Ok(_guard) = SLOT.try_lock()
                    {
                        let result = run(text.as_bytes(), ts, &cancel).map(|mut symbols| {
                            outline_tree::keyed(&mut symbols, "");
                            Arc::new(symbols)
                        });
                        let _ = ready.set(result);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            });
        if launched.is_err() {
            let _ = pending.ready.set(Err(()));
        }
        pending
    }
    pub(crate) fn result(&self) -> Option<&ResultTree> {
        self.ready.get()
    }
}

fn run(text: &[u8], ts: bool, cancel: &AtomicBool) -> Result<Vec<Symbol>, ()> {
    if text.len() > crate::files::MAX_TEXT {
        return Err(());
    }
    let mut cmd = Command::new(std::env::current_exe().map_err(|_| ())?);
    crate::agent_env::clean_command(&mut cmd);
    cmd.args(["--script-outline-worker", if ts { "ts" } else { "js" }])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = crate::pdf::Process(cmd.spawn().map_err(|_| ())?);
    #[cfg(windows)]
    let _job = crate::pdf::Job::attach(&child.0).map_err(|_| ())?;
    let mut input = child.0.stdin.take().ok_or(())?;
    let output = child.0.stdout.take().ok_or(())?;
    let buffer = std::thread::scope(|scope| {
        let writer = scope.spawn(move || input.write_all(text));
        let reader = scope.spawn(move || {
            let mut bytes = Vec::new();
            output.take(MAX_OUTPUT as u64 + 1).read_to_end(&mut bytes)?;
            Ok::<_, std::io::Error>(bytes)
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let success = loop {
            match child.0.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) if !cancel.load(Ordering::Acquire) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => break false,
            }
        };
        // Close the pipes before joining the threads on a cancellation or timeout.
        drop(child);
        if !success {
            return Err(());
        }
        writer.join().map_err(|_| ())?.map_err(|_| ())?;
        reader.join().map_err(|_| ())?.map_err(|_| ())
    })?;
    if buffer.len() > MAX_OUTPUT {
        return Err(());
    }
    serde_json::from_slice(&buffer).map_err(|_| ())
}

pub(crate) fn worker() -> Result<(), ()> {
    let ts = match std::env::args().nth(2).as_deref() {
        Some("ts") => true,
        Some("js") => false,
        _ => return Err(()),
    };
    let mut source = String::new();
    std::io::stdin()
        .take(crate::files::MAX_TEXT as u64 + 1)
        .read_to_string(&mut source)
        .map_err(|_| ())?;
    if source.len() > crate::files::MAX_TEXT {
        return Err(());
    }
    let symbols = crate::script_outline::document_symbols(&source, ts);
    serde_json::to_writer(std::io::stdout().lock(), &symbols).map_err(|_| ())
}
