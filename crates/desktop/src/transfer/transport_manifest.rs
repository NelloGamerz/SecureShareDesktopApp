//! The optional transport manifest the receiver attaches to an ACCEPT, and the
//! sender's choice of path from it.
//!
//! The central backend carries the manifest from the receiver to the sender as a
//! sibling of the frozen tunnel fields in the WebSocket `START_TRANSFER`
//! message. Nothing here changes those fields: when no manifest is present the
//! sender takes exactly the tunnel path it took before this module existed.
//!
//! Only LAN is produced today. The P2P and BLE sections need `iroh`/`webrtc` and
//! the BLE plugin, which are not dependencies of this build, so those
//! capabilities are reported `false`, their sections are omitted, and the
//! selector cannot dial them. Adding one is a builder function plus a flag.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use vilsend_core::TransportKind;

use crate::transfer::constants::RECEIVER_PORT;

/// The backend rejects a manifest larger than this, so the receiver does not
/// send one.
pub const MANIFEST_LIMIT_BYTES: usize = 16 * 1024;

/// A probe that runs longer than this counts as unavailable: accepting must not
/// hang on it.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// A direct LAN dial gets this long before the sender moves on.
const LAN_DIAL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransportManifest {
    #[serde(default)]
    pub manifest_version: Option<u8>,
    #[serde(default)]
    pub capabilities: Option<ManifestCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan: Option<LanSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p2p: Option<P2pSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ble: Option<BleSection>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestCapabilities {
    #[serde(default)]
    pub p2p: bool,
    #[serde(default)]
    pub lan: bool,
    #[serde(default)]
    pub ble: bool,
    #[serde(default)]
    pub preferred_order: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanSection {
    #[serde(default)]
    pub endpoints: Vec<LanEndpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mdns_service: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanEndpoint {
    pub ip: String,
    pub port: u16,
}

/// Parsed for forward compatibility. The sender cannot dial P2P in this build.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct P2pSection {
    #[serde(default)]
    pub peer_id: Option<String>,
    #[serde(default)]
    pub sdp_offer: Option<String>,
    #[serde(default)]
    pub ice_candidates: Vec<String>,
    #[serde(default)]
    pub relay_url: Option<String>,
}

/// Parsed for forward compatibility. The sender cannot dial BLE in this build.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BleSection {
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub service_uuid: Option<String>,
    #[serde(default)]
    pub char_uuid: Option<String>,
    #[serde(default)]
    pub mtu: Option<u16>,
}

/// Probes local capabilities for the accept request.
///
/// Never fails and never blocks longer than [`PROBE_TIMEOUT`]: a probe that
/// errors or runs long only costs that capability. Returns `None` when nothing
/// usable was found or the encoded manifest would exceed the backend limit.
pub async fn build_manifest() -> Option<serde_json::Value> {
    let lan = match tokio::time::timeout(PROBE_TIMEOUT, probe_lan()).await {
        Ok(section) => section,
        Err(_) => {
            tracing::warn!("LAN capability probe timed out; reporting LAN unavailable");
            None
        }
    };

    let manifest = TransportManifest {
        manifest_version: Some(1),
        capabilities: Some(ManifestCapabilities {
            p2p: false,
            lan: lan.is_some(),
            ble: false,
            preferred_order: vec!["lan".into(), "p2p".into(), "ble".into()],
        }),
        lan,
        p2p: None,
        ble: None,
    };

    let value = serde_json::to_value(&manifest).ok()?;
    let size = serde_json::to_vec(&value).ok()?.len();

    if size > MANIFEST_LIMIT_BYTES {
        tracing::warn!(
            bytes = size,
            "transport manifest exceeds 16 KiB; omitting it"
        );
        return None;
    }

    Some(value)
}

async fn probe_lan() -> Option<LanSection> {
    let ip = local_ip().await?;

    Some(LanSection {
        endpoints: vec![LanEndpoint {
            ip: ip.to_string(),
            port: RECEIVER_PORT,
        }],
        mdns_service: Some("vilsend._tcp.local".into()),
    })
}

/// The address of the interface the default route uses.
///
/// A UDP `connect` sends no packet; it only makes the kernel choose a source
/// address. Without a route there is no LAN address to advertise.
///
/// shortcut: one IPv4 address and no netmask (no `if-addrs` dependency).
/// Upgrade when the selector must enumerate several NICs or compare subnets.
async fn local_ip() -> Option<std::net::IpAddr> {
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await.ok()?;
    socket.connect("8.8.8.8:80").await.ok()?;
    socket.local_addr().ok().map(|address| address.ip())
}

/// The path the sender chose from a manifest.
#[derive(Debug, Clone)]
pub struct SelectedTransport {
    pub kind: TransportKind,
    /// Endpoint for a direct path. `None` means keep the tunnel endpoint from
    /// the WebSocket message.
    pub direct_endpoint: Option<String>,
}

/// Chooses a path for the sender.
///
/// Returns `None` when the manifest offers nothing this build can dial, which
/// tells the caller to keep the tunnel fields exactly as they arrived.
///
/// `preferred_order` is accepted but not consulted: with only LAN and the
/// tunnel implemented there is nothing for it to order. Consult it here when
/// P2P/BLE dialling lands.
pub async fn select(manifest: &TransportManifest) -> Option<SelectedTransport> {
    let capabilities = manifest.capabilities.as_ref()?;

    if capabilities.lan {
        for endpoint in manifest.lan.as_ref().map(|lan| &lan.endpoints).into_iter().flatten() {
            if lan_reachable(&endpoint.ip, endpoint.port).await {
                return Some(SelectedTransport {
                    kind: TransportKind::Lan,
                    direct_endpoint: Some(format!("http://{}:{}", endpoint.ip, endpoint.port)),
                });
            }
        }
    }

    // P2P and BLE are not dialled: `iroh`/`webrtc` and the BLE plugin are not
    // dependencies of this build, so a manifest advertising them is honoured by
    // falling back to the tunnel the sender already has.
    // shortcut: dial P2P/BLE here when those crates land.

    None
}

/// A TCP connect is the whole LAN test: the receiver already listens on its
/// transfer port.
///
/// shortcut: no subnet comparison — there is no netmask without `if-addrs`, and
/// a direct dial that answers is stronger evidence than a guessed /24.
async fn lan_reachable(ip: &str, port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            LAN_DIAL_TIMEOUT,
            tokio::net::TcpStream::connect((ip, port))
        )
        .await,
        Ok(Ok(_))
    )
}

/// The label the webview shows for a chosen path.
pub fn transport_label(kind: TransportKind) -> &'static str {
    match kind {
        TransportKind::Lan => "LAN",
        TransportKind::P2p => "P2P",
        TransportKind::Bluetooth => "Bluetooth",
        TransportKind::Relay => "Relay",
        TransportKind::Tunnel => "Tunnel",
        TransportKind::Memory => "Memory",
        TransportKind::Usb => "USB",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_without_lan_selects_nothing() {
        let manifest: TransportManifest = serde_json::from_value(serde_json::json!({
            "manifest_version": 1,
            "capabilities": { "lan": false, "p2p": true, "ble": false }
        }))
        .unwrap();

        assert!(tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(select(&manifest))
            .is_none());
    }

    #[test]
    fn a_malformed_manifest_is_a_parse_error_not_a_panic() {
        let result = serde_json::from_value::<TransportManifest>(serde_json::json!({
            "capabilities": { "lan": "yes" }
        }));

        assert!(result.is_err());
    }

    #[test]
    fn an_empty_manifest_has_no_capabilities() {
        let manifest: TransportManifest =
            serde_json::from_value(serde_json::json!({})).unwrap();

        assert!(tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(select(&manifest))
            .is_none());
    }

    #[test]
    fn labels_are_the_ones_the_ui_shows() {
        assert_eq!(transport_label(TransportKind::Lan), "LAN");
        assert_eq!(transport_label(TransportKind::P2p), "P2P");
        assert_eq!(transport_label(TransportKind::Bluetooth), "Bluetooth");
        assert_eq!(transport_label(TransportKind::Relay), "Relay");
        assert_eq!(transport_label(TransportKind::Tunnel), "Tunnel");
    }
}
