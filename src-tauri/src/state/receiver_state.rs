use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ReceiverState {
    pub root: PathBuf,
    pub guard: Arc<tokio::sync::Mutex<()>>,
    pub keys: Arc<RwLock<HashMap<String, [u8; 32]>>>,
}

impl ReceiverState {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            guard: Arc::new(tokio::sync::Mutex::new(())),
            keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}
