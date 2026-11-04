use super::local::LocalPty;

pub(super) enum Backend {
    Local(LocalPty),
    #[cfg(windows)]
    Native(Box<super::native::Native>),
}
impl Backend {
    pub fn stop_for_restart(&mut self) -> Result<(), String> {
        match self {
            Self::Local(local) => local.stop_for_restart(),
            #[cfg(windows)]
            Self::Native(_) => Err("Remote restart requires an explicit new session".into()),
        }
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
        match self {
            Self::Local(local) => local.send(bytes),
            #[cfg(windows)]
            Self::Native(native) => native.send(bytes),
        }
    }
    /// Only a local resize changes the emulator immediately.
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<bool, String> {
        match self {
            Self::Local(local) => local.resize(rows, cols).map(|()| true),
            #[cfg(windows)]
            Self::Native(native) => native.resize(rows, cols).map(|()| false),
        }
    }
    pub fn writable(&self) -> bool {
        match self {
            Self::Local(_) => true,
            #[cfg(windows)]
            Self::Native(native) => native.writable(),
        }
    }
    pub fn try_output(&self) -> Option<Vec<u8>> {
        match self {
            Self::Local(local) => local.try_output(),
            #[cfg(windows)]
            Self::Native(_) => None,
        }
    }
    pub fn exit_status(&mut self) -> Option<portable_pty::ExitStatus> {
        match self {
            Self::Local(local) => local.exit_status(),
            #[cfg(windows)]
            Self::Native(_) => None,
        }
    }
    #[cfg(all(test, windows))]
    pub fn process_tree_exited(&mut self) -> bool {
        match self {
            Self::Local(local) => local.process_tree_exited(),
            Self::Native(_) => false,
        }
    }
    #[cfg(test)]
    pub fn replace_input(&mut self, input: std::sync::mpsc::SyncSender<Vec<u8>>) {
        match self {
            Self::Local(local) => local.replace_input(input),
            #[cfg(windows)]
            Self::Native(_) => panic!("local fixture required"),
        }
    }
    #[cfg(test)]
    pub fn recv_output(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Vec<u8>, std::sync::mpsc::RecvTimeoutError> {
        match self {
            Self::Local(local) => local.recv_output(timeout),
            #[cfg(windows)]
            Self::Native(_) => Err(std::sync::mpsc::RecvTimeoutError::Disconnected),
        }
    }
}
