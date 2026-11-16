//! Bounded async body collection shared by HTTP and synthetic stream fixtures.
use bytes::Bytes;
use std::future::Future;
pub(super) trait Chunks: Send {
    fn next(&mut self) -> impl Future<Output = Result<Option<Bytes>, String>> + Send;
}
impl Chunks for reqwest::Response {
    async fn next(&mut self) -> Result<Option<Bytes>, String> {
        self.chunk()
            .await
            .map_err(|_| "Cannot read Linear response".into())
    }
}
pub(super) async fn collect(mut source: impl Chunks, limit: usize) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    while let Some(chunk) = source.next().await? {
        if chunk.len() > limit - result.len() {
            return Err("Linear data exceeds size limit".into());
        }
        result.extend_from_slice(&chunk);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };
    struct Source {
        chunks: VecDeque<Result<Bytes, String>>,
        reads: Arc<AtomicUsize>,
    }
    impl Chunks for Source {
        async fn next(&mut self) -> Result<Option<Bytes>, String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.chunks.pop_front().transpose()
        }
    }
    #[tokio::test]
    async fn unknown_length_stream_is_bounded_before_appending_and_errors_propagate() {
        let reads = Arc::new(AtomicUsize::new(0));
        let source = Source {
            chunks: vec![
                Ok(Bytes::from_static(b"1234")),
                Ok(Bytes::from_static(b"56789")),
                Err("must not read".into()),
            ]
            .into(),
            reads: reads.clone(),
        };
        assert!(collect(source, 8).await.is_err());
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        let source = Source {
            chunks: vec![
                Ok(Bytes::from_static(b"1234")),
                Ok(Bytes::from_static(b"5678")),
            ]
            .into(),
            reads: reads.clone(),
        };
        assert_eq!(collect(source, 8).await.unwrap(), b"12345678");
        let source = Source {
            chunks: vec![Err("synthetic read failure".into())].into(),
            reads,
        };
        assert!(collect(source, 8).await.is_err());
    }
    struct Pending {
        started: Option<tokio::sync::oneshot::Sender<()>>,
        dropped: Arc<AtomicBool>,
    }
    impl Drop for Pending {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }
    impl Chunks for Pending {
        async fn next(&mut self) -> Result<Option<Bytes>, String> {
            let _ = self.started.take().unwrap().send(());
            std::future::pending().await
        }
    }
    #[tokio::test]
    async fn closing_owner_drops_pending_body_without_waiting_for_io() {
        let owner = super::super::request::Generation::default();
        let mut scope = owner.scope();
        let (started, wait) = tokio::sync::oneshot::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        let source = Pending {
            started: Some(started),
            dropped: dropped.clone(),
        };
        let job = tokio::spawn(async move { scope.run(collect(source, 8)).await });
        wait.await.unwrap();
        drop(owner);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), job)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(dropped.load(Ordering::SeqCst));
    }
}
