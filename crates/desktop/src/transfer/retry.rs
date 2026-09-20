use std::time::Duration;
pub fn delay(attempt: u32) -> Duration {
    Duration::from_secs(2_u64.saturating_pow(attempt.min(3)))
}
