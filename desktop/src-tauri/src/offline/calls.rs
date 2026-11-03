use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Calls {
    active: Option<u64>,
    finished: Option<Instant>,
}

impl Calls {
    pub(super) fn begin(&mut self, address: u64, now: Instant) -> bool {
        // A connectable advertiser can rotate its address during one call.
        // Serialize setup globally; an address is not a stable caller ID.
        if self.active.is_some()
            || self
                .finished
                .is_some_and(|t| now.duration_since(t) < Duration::from_secs(2))
        {
            return false;
        }
        self.active = Some(address);
        true
    }

    pub(super) fn finish(&mut self, address: u64, now: Instant) {
        if self.active == Some(address) {
            self.active = None;
            self.finished = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_running_call_cannot_be_started_twice() {
        let mut calls = Calls::default();
        let now = Instant::now();
        assert!(calls.begin(1, now));
        assert!(!calls.begin(1, now + Duration::from_secs(91)));
        assert!(
            !calls.begin(2, now + Duration::from_secs(91)),
            "a rotated BLE address must not start an overlapping setup"
        );
    }

    #[test]
    fn a_finished_call_can_be_retried_after_the_cooldown() {
        let mut calls = Calls::default();
        let now = Instant::now();
        assert!(calls.begin(1, now));
        calls.finish(1, now + Duration::from_secs(60));
        assert!(!calls.begin(1, now + Duration::from_secs(61)));
        assert!(
            calls.begin(1, now + Duration::from_secs(62)),
            "a deliberate retry must not wait another 30 seconds"
        );
    }
}
