use std::sync::Arc;

use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::websocket::WebSocketSender;
use serde_json::json;


pub async fn heartbeat_loop(
    sender: Arc<WebSocketSender>,
    shutdown: CancellationToken,
    interval_secs: u64,
) -> Result<(), AppError> {

    let interval = Duration::from_secs(interval_secs.max(1));

    loop {
        tokio::select! {

            _ = shutdown.cancelled() => {
                break;
            }

            _ = sleep(interval) => {

                let ping_message = json!({
                    "type": "PING"
                });


                let _ = sender.send(
                    tokio_tungstenite::tungstenite::Message::Text(
                        ping_message.to_string().into()
                    )
                );
            }
        }
    }

    Ok(())
}