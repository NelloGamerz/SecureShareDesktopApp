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
                let heartbeat = crate::websocket::is_heartbeat(&message);

                if !heartbeat {
                    println!("RECEIVED FRAME: {:?}", message);
                }

                if let Message::Text(text) = message {
                    if !heartbeat {
                        println!("TEXT: {}", text);
                    }

                    match serde_json::from_str::<ServerCommand>(&text) {
                        Ok(command) => {
                            /*
                             * One command failing is not a reason to lose the
                             * socket: a refused `START_TRANSFER` would otherwise
                             * tear down the command channel, and the control
                             * plane does not resend it.
                             */
                            if let Err(error) = dispatcher.emit_command(command).await {
                                tracing::warn!(
                                    target: "websocket",
                                    event = "command_failed",
                                    %error,
                                    "a websocket command failed; the connection stays up"
                                );
                            }
                        }

                        Err(err) => {
                            tracing::warn!("invalid websocket command: {}", err);
                        }
                    }
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
