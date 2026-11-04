//! A create key is reusable only during this desktop run and well inside the
//! daemon's ten-minute deduplication lifetime. Never serialize this permission.
use std::time::{Duration, Instant};
use tessel_host_protocol::Epoch;

const RETRY_WINDOW: Duration = Duration::from_secs(8 * 60);
const MAX_REQUEST: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    EpochChanged,
    Expired,
    RequestBudget,
}

/// Construct immediately before the first create request, never from saved
/// metadata. Restored pending records require explicit listing/attachment.
pub struct Attempt {
    epoch: Epoch,
    first_send: Instant,
}
impl Attempt {
    pub fn begin(epoch: Epoch, now: Instant) -> Self {
        Self {
            epoch,
            first_send: now,
        }
    }
    /// The request timeout is included in the deadline, leaving at least two
    /// minutes before the daemon could forget the original key.
    pub fn check(
        &self,
        current_epoch: &Epoch,
        now: Instant,
        request_timeout: Duration,
    ) -> Result<(), Refused> {
        if current_epoch != &self.epoch {
            return Err(Refused::EpochChanged);
        }
        if request_timeout.is_zero() || request_timeout > MAX_REQUEST {
            return Err(Refused::RequestBudget);
        }
        let elapsed = now
            .checked_duration_since(self.first_send)
            .ok_or(Refused::Expired)?;
        if elapsed
            .checked_add(request_timeout)
            .is_none_or(|end| end >= RETRY_WINDOW)
        {
            return Err(Refused::Expired);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadline_includes_the_whole_request_and_never_uses_wall_clock() {
        let start = Instant::now();
        let epoch = Epoch::from_bytes([1; 16]);
        let attempt = Attempt::begin(epoch.clone(), start);
        let timeout = Duration::from_secs(15);
        assert_eq!(attempt.check(&epoch, start, timeout), Ok(()));
        assert_eq!(
            attempt.check(&epoch, start + RETRY_WINDOW - timeout, timeout),
            Err(Refused::Expired)
        );
        assert_eq!(
            attempt.check(&epoch, start + RETRY_WINDOW, timeout),
            Err(Refused::Expired)
        );
        if let Some(before) = start.checked_sub(Duration::from_secs(1)) {
            assert_eq!(
                attempt.check(&epoch, before, timeout),
                Err(Refused::Expired)
            );
        }
    }
    #[test]
    fn changed_epoch_or_unbounded_request_cannot_reuse_the_key() {
        let start = Instant::now();
        let epoch = Epoch::from_bytes([1; 16]);
        let attempt = Attempt::begin(epoch.clone(), start);
        assert_eq!(
            attempt.check(&Epoch::from_bytes([2; 16]), start, Duration::from_secs(1)),
            Err(Refused::EpochChanged)
        );
        for budget in [Duration::ZERO, MAX_REQUEST + Duration::from_secs(1)] {
            assert_eq!(
                attempt.check(&epoch, start, budget),
                Err(Refused::RequestBudget)
            );
        }
    }
}
