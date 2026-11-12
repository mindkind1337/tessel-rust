//! Cancellation of a guest transport without joining a network thread on the UI thread.
use super::{GuestPhase, GuestSnapshot};
use std::{
    net::{Shutdown, TcpStream},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Default)]
pub(super) struct GuestStop {
    stopped: AtomicBool,
    socket: Mutex<Option<TcpStream>>,
}
impl GuestStop {
    pub(super) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
    pub(super) fn register(&self, stream: &TcpStream) -> Result<(), String> {
        let mut socket = self.socket.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_stopped() {
            return Err("Session closed.".into());
        }
        *socket = Some(stream.try_clone().map_err(|e| e.to_string())?);
        Ok(())
    }
    pub(super) fn cancel(&self) {
        let mut socket = self.socket.lock().unwrap_or_else(|e| e.into_inner());
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(stream) = socket.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
    pub(super) fn clear_socket(&self) {
        self.socket.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}
pub(super) fn active_state<'a>(
    state: &'a Mutex<GuestSnapshot>,
    stop: &GuestStop,
) -> Option<MutexGuard<'a, GuestSnapshot>> {
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    (!stop.is_stopped()).then_some(guard)
}
pub(super) fn mark_left(state: &Mutex<GuestSnapshot>) {
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    let host_name = match &state.phase {
        GuestPhase::Connecting => String::new(),
        GuestPhase::WaitingForHost { host_name, .. }
        | GuestPhase::Joined { host_name, .. }
        | GuestPhase::Ended { host_name, .. } => host_name.clone(),
    };
    state.phase = GuestPhase::Ended {
        reason: "left".into(),
        host_name,
    };
    state.last_error.clear();
}
pub(super) struct SocketLifetime<'a>(pub &'a GuestStop);
impl Drop for SocketLifetime<'_> {
    fn drop(&mut self) {
        self.0.clear_socket();
    }
}
