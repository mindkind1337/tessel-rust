//! Account-generation cancellation. UI delivery and durable commits must recheck
//! the scope too: completing a request is not permission to apply it indefinitely.
use std::future::Future;
use tokio::sync::watch;

pub struct Generation {
    sender: watch::Sender<u64>,
}
impl Default for Generation {
    fn default() -> Self {
        Self {
            sender: watch::channel(0).0,
        }
    }
}
impl Generation {
    pub fn scope(&self) -> Scope {
        Scope {
            generation: *self.sender.borrow(),
            receiver: self.sender.subscribe(),
        }
    }
    pub fn advance(&mut self) -> Result<Scope, String> {
        let next = self
            .sender
            .borrow()
            .checked_add(1)
            .ok_or("Linear request identity exhausted")?;
        self.sender.send_replace(next);
        Ok(self.scope())
    }
}
#[derive(Clone)]
pub struct Scope {
    generation: u64,
    receiver: watch::Receiver<u64>,
}
impl Scope {
    pub fn is_current(&self) -> bool {
        self.receiver.has_changed().is_ok() && *self.receiver.borrow() == self.generation
    }
    pub async fn run<T>(
        &mut self,
        future: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        // Check the value while marking its version seen. A separate pre-check
        // could miss an invalidation between checking and borrow_and_update.
        let observed = *self.receiver.borrow_and_update();
        if observed != self.generation || self.receiver.has_changed().is_err() {
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
}
