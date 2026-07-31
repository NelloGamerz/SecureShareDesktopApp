use crate::error::AppError;
use crate::models::DeviceInfo;

use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::header},
};

pub type WebSocketConnection =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub struct WebSocketClient {
    url: String,
    token: Option<String>,
    device_info: DeviceInfo,
}

impl WebSocketClient {
    pub fn new(url: String, token: Option<String>, device_info: DeviceInfo) -> Self {
        Self {
            url,
            token,
            device_info,
        }
    }

    pub async fn connect(&self) -> Result<WebSocketConnection, AppError> {
        println!("========== WS CONNECT START ==========");
        println!("URL: {}", self.url);

        println!("DEVICE HEADER: {}", self.device_info.device_identifier);

        let mut request = self
            .url
            .clone()
            .into_client_request()
            .map_err(|e| AppError::network(format!("invalid websocket url: {e}")))?;

        if let Some(token) = &self.token {
            println!("ADDING CLERK AUTHORIZATION HEADER");

            let auth_value = format!("Bearer {}", token);

            request.headers_mut().insert(
                header::AUTHORIZATION,
                auth_value
                    .parse()
                    .map_err(|e| AppError::network(format!("invalid authorization header: {e}")))?,
            );
        }

        // Device information headers
        println!(
            "ADDING DEVICE IDENTIFIER: {}",
            self.device_info.device_identifier
        );

        request.headers_mut().insert(
            "x-device-id",
            self.device_info
                .device_identifier
                .parse()
                .map_err(|e| AppError::network(format!("invalid device id: {e}")))?,
        );

        println!("FINAL WS HEADERS: {:?}", request.headers());

        println!("CALLING connect_async");

        let (stream, response) = connect_async(request).await.map_err(|e| {
            println!("CONNECT FAILED: {:?}", e);

            AppError::network(format!("failed websocket connection: {e}"))
        })?;

        println!("CONNECTED. HTTP STATUS: {}", response.status());

        Ok(stream)
    }
}
