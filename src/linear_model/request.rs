//! Account-generation cancellation and serialized local commits. Completing a
//! request alone is not permission to apply it; use commit at the delivery boundary.
use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;

struct Authority {
    gate: Mutex<()>,
    sender: watch::Sender<u64>,
    closed: AtomicBool,
}
pub struct Generation {
    authority: Arc<Authority>,
}
impl Default for Generation {
    fn default() -> Self {
        Self {
            authority: Arc::new(Authority {
                gate: Mutex::new(()),
                sender: watch::channel(0).0,
                closed: AtomicBool::new(false),
            }),
        }
    }
}
impl Generation {
    pub fn scope(&self) -> Scope {
        Scope::new(self.authority.clone())
    }
    pub fn advance(&mut self) -> Result<Scope, String> {
        let _guard = self
            .authority
            .gate
            .lock()
            .map_err(|_| "Linear account authority is unavailable")?;
        let next = self
            .authority
            .sender
            .borrow()
            .checked_add(1)
            .ok_or("Linear request identity exhausted")?;
        self.authority.sender.send_replace(next);
        Ok(self.scope())
    }
}
impl Drop for Generation {
    fn drop(&mut self) {
        let _guard = self
            .authority
            .gate
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.authority.closed.store(true, Ordering::Release);
        // Scopes keep the authority alive, so notify explicitly on owner shutdown.
        self.authority.sender.send_modify(|_| {});
    }
}
#[derive(Clone)]
pub struct Scope {
    generation: u64,
    receiver: watch::Receiver<u64>,
    authority: Arc<Authority>,
}
impl Scope {
    fn new(authority: Arc<Authority>) -> Self {
        let generation = *authority.sender.borrow();
        let receiver = authority.sender.subscribe();
        Self {
            generation,
            receiver,
            authority,
        }
    }
    pub fn is_current(&self) -> bool {
        !self.authority.closed.load(Ordering::Acquire)
            && !self.authority.gate.is_poisoned()
            && *self.receiver.borrow() == self.generation
    }
    /// Serialize a synchronous local commit or UI delivery with account changes.
    /// The callback must not re-enter this authority or wait for asynchronous work.
    pub fn commit<T>(&self, apply: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let _guard = self
            .authority
            .gate
            .lock()
            .map_err(|_| "Linear account authority is unavailable")?;
        if !self.is_current() {
            return Err("Linear request cancelled".into());
        }
        apply()
    }
    /// A successful durable account change invalidates old reads before unlocking.
    /// Failure keeps the old generation, allowing the previous connection to survive.
    pub fn commit_and_advance<T>(
        &self,
        apply: impl FnOnce() -> Result<T, String>,
    ) -> Result<(T, Scope), String> {
        let _guard = self
            .authority
            .gate
            .lock()
            .map_err(|_| "Linear account authority is unavailable")?;
        if !self.is_current() {
            return Err("Linear request cancelled".into());
        }
        let next = self
            .generation
            .checked_add(1)
            .ok_or("Linear request identity exhausted")?;
        let value = apply()?;
        self.authority.sender.send_replace(next);
        Ok((value, Scope::new(self.authority.clone())))
    }
    pub async fn run<T>(
        &mut self,
        future: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        // Check the value while marking its version seen. A separate pre-check
        // could miss an invalidation between checking and borrow_and_update.
        let observed = *self.receiver.borrow_and_update();
        if observed != self.generation || !self.is_current() {
            return Err("Linear request cancelled".into());
        }
        tokio::pin!(future);
        tokio::select! {
            biased;
            _=self.receiver.changed()=>Err("Linear request cancelled".into()),
            result=&mut future=>{
                if self.is_current(){result}else{Err("Linear request cancelled".into())}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    #[tokio::test]
    async fn account_change_drops_inflight_future_and_rejects_queued_old_request() {
        let mut generation = Generation::default();
        let mut scope = generation.scope();
        let mut queued = scope.clone();
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        let (started, wait) = tokio::sync::oneshot::channel();
        let running = tokio::spawn(async move {
            scope
                .run(async move {
                    let _guard = guard;
                    let _ = started.send(());
                    std::future::pending::<()>().await;
                    Ok(())
                })
                .await
        });
        wait.await.unwrap();
        let mut fresh = generation.advance().unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), running)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(dropped.load(Ordering::SeqCst));
        assert!(
            queued
                .run(async {
                    panic!("stale request must not be polled");
                    #[allow(unreachable_code)]
                    Ok(())
                })
                .await
                .is_err()
        );
        assert_eq!(fresh.run(async { Ok(42) }).await.unwrap(), 42);
        generation.advance().unwrap();
        assert!(
            !fresh.is_current(),
            "queued UI delivery also needs a fresh check"
        );
    }
    #[tokio::test]
    async fn owner_drop_cancels_and_a_new_owner_cannot_revive_old_scopes() {
        let owner = Generation::default();
        let mut scope = owner.scope();
        drop(owner);
        let replacement = Generation::default();
        assert!(replacement.scope().is_current());
        assert!(!scope.is_current());
        assert!(scope.run(async { Ok(()) }).await.is_err());
    }
    #[tokio::test]
    async fn invalidation_wins_even_when_result_is_ready() {
        let mut owner = Generation::default();
        let mut scope = owner.scope();
        let result = scope
            .run(async {
                owner.advance().unwrap();
                Ok("stale")
            })
            .await;
        assert!(result.is_err());
    }
    #[test]
    fn failed_account_commit_retains_authority_and_success_revokes_previous_reads() {
        let mut owner = Generation::default();
        let scope = owner.scope();
        assert!(
            scope
                .commit_and_advance(|| Err::<(), _>("synthetic storage failure".into()))
                .is_err()
        );
        assert!(scope.is_current());
        let (value, current) = scope.commit_and_advance(|| Ok("saved")).unwrap();
        assert_eq!(value, "saved");
        assert!(!scope.is_current());
        assert!(current.is_current());
        assert!(
            scope
                .commit::<()>(|| panic!("stale delivery must not apply"))
                .is_err()
        );
        owner.advance().unwrap();
        assert!(
            current
                .commit_and_advance::<()>(|| panic!("stale save must not run"))
                .is_err()
        );
    }
    #[test]
    fn account_invalidation_waits_for_an_already_started_local_commit() {
        use std::{sync::mpsc, thread, time::Duration};
        let owner = Generation::default();
        let scope = owner.scope();
        let committing = scope.clone();
        let (entered, entry) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let commit = thread::spawn(move || {
            committing.commit(|| {
                entered.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok("committed")
            })
        });
        entry.recv_timeout(Duration::from_secs(1)).unwrap();
        let (attempted, attempt) = mpsc::channel();
        let (advanced, advance) = mpsc::channel();
        let invalidation = thread::spawn(move || {
            let mut owner = owner;
            attempted.send(()).unwrap();
            let next = owner.advance().unwrap();
            advanced.send(()).unwrap();
            (owner, next)
        });
        attempt.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(advance.try_recv().is_err());
        release.send(()).unwrap();
        assert_eq!(commit.join().unwrap().unwrap(), "committed");
        let (_owner, next) = invalidation.join().unwrap();
        assert!(!scope.is_current());
        assert!(next.is_current());
        assert!(
            scope
                .commit::<()>(|| panic!("invalidated result must not apply"))
                .is_err()
        );
    }
}
