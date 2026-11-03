use std::time::{Duration, Instant};

pub(super) struct ResponseBudget {
    total: Instant,
    idle: Instant,
    idle_span: Duration,
}
impl ResponseBudget {
    pub fn new(now: Instant) -> Self {
        Self::with_limits(now, Duration::from_secs(300), Duration::from_secs(120))
    }
    fn with_limits(now: Instant, total: Duration, idle_span: Duration) -> Self {
        let total = now + total;
        Self {
            total,
            idle: total,
            idle_span,
        }
    }
    pub fn deadline(&self) -> Instant {
        self.total.min(self.idle)
    }
    pub fn progress(&mut self, now: Instant) -> bool {
        if now >= self.deadline() {
            return false;
        }
        self.idle = self.total.min(now + self.idle_span);
        true
    }
    pub fn expired_kind(&self, now: Instant) -> super::ModelError {
        if now >= self.total {
            super::ModelError::TotalTimeout
        } else {
            super::ModelError::IdleTimeout
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_first_data_uses_total_not_active_idle() {
        let now = Instant::now();
        let mut b =
            ResponseBudget::with_limits(now, Duration::from_secs(300), Duration::from_secs(120));
        assert_eq!(b.deadline(), now + Duration::from_secs(300));
        assert!(b.progress(now + Duration::from_secs(150)));
        assert_eq!(b.deadline(), now + Duration::from_secs(270));
    }
    #[test]
    fn progress_cannot_extend_total_or_resurrect_expiration() {
        let now = Instant::now();
        let mut b = ResponseBudget::new(now);
        assert!(b.progress(now + Duration::from_secs(100)));
        assert!(b.progress(now + Duration::from_secs(210)));
        assert_eq!(b.deadline(), now + Duration::from_secs(300));
        assert!(!b.progress(now + Duration::from_secs(300)));
        assert_eq!(
            b.expired_kind(now + Duration::from_secs(300)),
            super::super::ModelError::TotalTimeout
        );
    }
    #[test]
    fn active_idle_is_distinct_from_total() {
        let now = Instant::now();
        let mut b = ResponseBudget::new(now);
        assert!(b.progress(now));
        assert_eq!(
            b.expired_kind(now + Duration::from_secs(120)),
            super::super::ModelError::IdleTimeout
        );
        assert!(!b.progress(now + Duration::from_secs(120)));
    }
}
