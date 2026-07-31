pub struct WebSocketManager {
    sender: Mutex<Option<mpsc::Sender<Message>>>,
    shutdown: CancellationToken,
}