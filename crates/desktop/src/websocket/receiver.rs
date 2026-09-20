use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

use crate::error::AppError;
use crate::events::EventDispatcher;
use crate::websocket::ServerCommand;

pub async fn receive_loop(
    mut incoming: futures_util::stream::SplitStream<crate::websocket::client::WebSocketConnection>,
    dispatcher: std::sync::Arc<EventDispatcher>,
) -> Result<(), AppError> {
    while let Some(message) = incoming.next().await {
        match message {
            Ok(message) => {
                println!("RECEIVED FRAME: {:?}", message);

                match message {
                    Message::Text(text) => {
                        println!("TEXT: {}", text);

                        match serde_json::from_str::<ServerCommand>(&text) {
                            Ok(command) => {
                                dispatcher.emit_command(command).await?;
                            }

                            Err(err) => {
                                tracing::warn!("invalid websocket command: {}", err);
                            }
                        }
                    }

                    _ => {}
                }
            }

            Err(err) => {
                println!("RECEIVE ERROR: {:?}", err);

                tracing::warn!(
                    target: "websocket",
                    event = "receive_error",
                    error = %err,
                    "websocket receive loop failed"
                );

                return Err(AppError::network(format!("websocket receive error: {err}")));
            }
        }
    }

    // Ok(())
    println!("WebSocket stream closed");

    Err(AppError::network("websocket connection closed".to_string()))
}
