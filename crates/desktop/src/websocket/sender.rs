use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone)]
pub struct WebSocketSender {
    tx: mpsc::UnboundedSender<Message>,
}

impl WebSocketSender {
    pub fn new(tx: mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }

    pub fn send(&self, message: Message) -> Result<(), String> {
        self.tx.send(message).map_err(|e| e.to_string())
    }
}
