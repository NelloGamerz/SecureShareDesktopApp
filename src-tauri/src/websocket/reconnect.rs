use std::time::Duration;

use tokio::time::sleep;

pub async fn backoff_delay(attempt: u32, initial_delay_ms: u64, max_delay_ms: u64) -> Duration {
    let delay_ms = initial_delay_ms.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
    let bounded = delay_ms.min(max_delay_ms);
    sleep(Duration::from_millis(bounded)).await;
    Duration::from_millis(bounded)
}
