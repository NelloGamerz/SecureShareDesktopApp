// use serde::{Deserialize, Serialize};

// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct WebSocketMessage {
//     pub r#type: String,
//     pub payload: serde_json::Value,
// }

// impl WebSocketMessage {
//     pub fn new(r#type: impl Into<String>, payload: serde_json::Value) -> Self {
//         Self {
//             r#type: r#type.into(),
//             payload,
//         }
//     }
// }
