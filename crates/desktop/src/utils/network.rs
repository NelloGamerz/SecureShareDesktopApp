use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

pub async fn has_internet() -> bool {
    let result = timeout(
        Duration::from_secs(3),
        TcpStream::connect(("1.1.1.1", 443)),
    )
    .await;

    result.is_ok() && result.unwrap().is_ok()
}