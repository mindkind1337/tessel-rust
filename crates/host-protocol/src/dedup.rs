use crate::ids::DedupKey;
use crate::wire::{ErrorCode, WireError};
use std::collections::HashMap;

/// What to do with an operation that carries a deduplication key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dedup<V> {
    /// First time: do it, then `finish` (or `abandon` if it had no effect at all).
    Fresh,
    /// Still being done by an earlier request: do not start it again.
    Running,
    /// Done before: answer with this outcome instead of doing it again.
    Finished(V),
}

#[derive(Clone, Debug)]
struct Entry<V> {
    stamp_ms: u64,
    outcome: Option<V>,
}

/// Outcomes of operations with an effect, by key, kept `ttl_ms` after they finish, across
/// connections. Bounded: when full of entries still within their time, a new key is refused
/// as busy rather than evicting one, so a retry within the time can never do it twice.
/// Times are milliseconds from a monotonic clock the caller reads.
#[derive(Clone, Debug)]
pub struct DedupCache<V> {
    capacity: usize,
    ttl_ms: u64,
    entries: HashMap<DedupKey, Entry<V>>,
}

impl<V: Clone> DedupCache<V> {
    pub fn new(capacity: usize, ttl_ms: u64) -> Self {
        Self {
            capacity,
            ttl_ms,
            entries: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn expire(&mut self, now_ms: u64) {
        let ttl = self.ttl_ms;
        self.entries.retain(|_, entry| {
            entry.outcome.is_none() || now_ms.saturating_sub(entry.stamp_ms) < ttl
        });
    }

    pub fn begin(&mut self, key: &DedupKey, now_ms: u64) -> Result<Dedup<V>, WireError> {
        self.expire(now_ms);
        if let Some(entry) = self.entries.get(key) {
            return Ok(match &entry.outcome {
                None => Dedup::Running,
                Some(outcome) => Dedup::Finished(outcome.clone()),
            });
        }
        if self.entries.len() >= self.capacity {
            return Err(WireError::new(ErrorCode::Busy));
        }
        self.entries.insert(
            key.clone(),
            Entry {
                stamp_ms: now_ms,
                outcome: None,
            },
        );
        Ok(Dedup::Fresh)
    }

    /// Records the outcome; its time starts now.
    pub fn finish(&mut self, key: &DedupKey, outcome: V, now_ms: u64) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.outcome = Some(outcome);
            entry.stamp_ms = now_ms;
        }
    }

    /// Forgets a key whose operation failed before any effect, so it may be tried again.
    pub fn abandon(&mut self, key: &DedupKey) {
        if matches!(self.entries.get(key), Some(Entry { outcome: None, .. })) {
            self.entries.remove(key);
        }
    }
}
