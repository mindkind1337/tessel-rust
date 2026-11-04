//! Local PTY ownership, bounded I/O and process-tree cleanup.
use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};

pub(super) struct LocalPty {
    master: Box<dyn MasterPty + Send>,
    input: Option<SyncSender<Vec<u8>>>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<Vec<u8>>,
}
impl LocalPty {
    pub fn spawn(command: CommandBuilder) -> Result<Self, Box<dyn std::error::Error>> {
        let pair = native_pty_system().openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        // Bounded queue prevents a noisy subprocess from consuming unlimited memory.
        let (sender, output) = mpsc::sync_channel(64);
        let (input, input_rx) = mpsc::sync_channel::<Vec<u8>>(32);
        thread::spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if writer
                    .write_all(&bytes)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if sender.send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Ok(Self {
            master: pair.master,
            input: Some(input),
            child,
            output,
        })
    }
    /// Called on the restart worker. No replacement may start until this succeeds.
    pub fn stop_for_restart(&mut self) -> Result<(), String> {
        if !cfg!(windows) {
            return Err("Verified terminal-tree restart is unavailable on this platform.".into());
        }
        self.child
            .kill()
            .map_err(|_| "Could not stop the terminal process group.")?;
        self.input.take();
        let (_, empty) = mpsc::channel();
        self.output = empty;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let tree_done = self
                .child
                .process_tree_exited()
                .map_err(|_| "Cannot verify terminal process-group termination.")?;
            let child_done = self
                .child
                .try_wait()
                .map_err(|_| "Cannot verify terminal process termination.")?
                .is_some();
            if tree_done && child_done {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err("Terminal processes have not stopped.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub fn try_output(&self) -> Option<Vec<u8>> {
        self.output.try_recv().ok()
    }
    pub fn exit_status(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().ok().flatten()
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.input
            .as_ref()
            .ok_or("Terminal input closed")?
            .try_send(bytes)
            .map_err(|_| "Terminal input is busy or closed; input was not sent.".into())
    }
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|_| "Cannot resize local terminal".into())
    }
    #[cfg(all(test, windows))]
    pub fn process_tree_exited(&mut self) -> bool {
        self.child.process_tree_exited().unwrap()
    }
    #[cfg(test)]
    pub fn recv_output(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Vec<u8>, mpsc::RecvTimeoutError> {
        self.output.recv_timeout(timeout)
    }
    #[cfg(test)]
    pub fn replace_input(&mut self, input: SyncSender<Vec<u8>>) {
        self.input = Some(input);
    }
}
impl Drop for LocalPty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        self.input.take();
        // Unblock output before closing ConPTY, including a full output queue.
        let (_, empty) = mpsc::channel();
        self.output = empty;
        let _ = self.child.wait();
    }
}
