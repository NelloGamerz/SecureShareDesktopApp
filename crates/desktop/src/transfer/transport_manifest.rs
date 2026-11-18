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

/// The smallest valid manifest: every capability false, no sections.
///
/// The backend records this differently from no manifest at all, which is why
/// an accept always carries one.
pub fn empty_manifest() -> serde_json::Value {
    serde_json::json!({
        "manifest_version": 1,
        "capabilities": { "p2p": false, "lan": false, "ble": false, "preferred_order": [] }
    })
}

/// Probes local capabilities for the accept request.
///
/// Never fails and never blocks longer than [`PROBE_TIMEOUT`]: a probe that
/// errors or runs long only costs that capability, and the manifest is still
/// sent with that capability false. A manifest too large for the backend is
/// replaced by [`empty_manifest`] rather than omitted.
pub async fn build_manifest() -> serde_json::Value {
    let manifest = probe().await;
    let lan_available = manifest.capabilities.as_ref().is_some_and(|caps| caps.lan);
    let endpoints = manifest.lan.as_ref().map(|lan| lan.endpoints.len()).unwrap_or(0);

    tracing::info!(
        "accept manifest: caps={lan},{p2p},{ble} lan_endpoints={endpoints}",
        lan = lan_available,
        p2p = false,
        ble = false,
        endpoints = endpoints
    );

    let value = match serde_json::to_value(&manifest) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "could not encode the transport manifest; sending an empty one");
            return empty_manifest();
        }
    };

    if serde_json::to_vec(&value).map(|bytes| bytes.len()).unwrap_or(0) > MANIFEST_LIMIT_BYTES {
        tracing::warn!("transport manifest exceeds 16 KiB; sending an empty one");
        return empty_manifest();
    }

    value
}

/// This machine's capabilities as a manifest, for the network report.
///
/// Same probe the accept path sends, without the size check: the report is a
/// local file, and the receiver's own copy of what it advertised costs nothing.
pub async fn probe() -> TransportManifest {
    let lan = match tokio::time::timeout(PROBE_TIMEOUT, probe_lan()).await {
        Ok(section) => section,
        Err(_) => {
            tracing::warn!("LAN capability probe timed out; reporting LAN unavailable");
            None
        }
    };

    // P2P and BLE are not implemented in this build, so they are never
    // available and never appear in the preferred order.
    let lan_available = lan.is_some();
    let mut preferred_order = Vec::new();

    if lan_available {
        preferred_order.push("lan".to_owned());
    }

    TransportManifest {
        manifest_version: Some(1),
        capabilities: Some(ManifestCapabilities {
            p2p: false,
            lan: lan_available,
            ble: false,
            preferred_order,
        }),
        lan,
        p2p: None,
        ble: None,
    }
}

/// Interface-name fragments whose addresses never reach a peer on the LAN:
/// loopback pseudo-adapters, container bridges, hypervisor host-only switches
/// and tunnel/tap devices. Matched case-insensitively.
const UNSUITABLE_INTERFACE_MARKERS: &[&str] = &[
    "loopback", "pseudo", "docker", "veth", "br-", "virbr", "vmnet", "vboxnet",
    "vethernet", "hyper-v", "virtual", "wsl", "tap", "tun", "utun", "awdl",
    "llw", "bluetooth",
];

async fn probe_lan() -> Option<LanSection> {
    // The default-route address is the best single guess, so it stays first
    // when enumeration finds it and remains the fallback when it finds nothing.
    let default = match local_ip().await {
        Some(std::net::IpAddr::V4(ip)) if is_dialable(ip) => Some(ip),
        _ => None,
    };

    let mut ips = discover_lan_ipv4s();

    match default {
        // Nothing enumerated: keep the previous single-endpoint behaviour
        // rather than reporting LAN unavailable.
        Some(ip) if ips.is_empty() => ips.push(ip),
        Some(ip) if ips.contains(&ip) => {
            ips.retain(|candidate| *candidate != ip);
            ips.insert(0, ip);
        }
        _ => {}
    }

    if ips.is_empty() {
        tracing::warn!("lan probe: no usable private ipv4 interface");
        return None;
    }

    tracing::info!(
        endpoints = ips.len(),
        "lan probe: advertising private interface addresses"
    );

    Some(LanSection {
        endpoints: ips
            .into_iter()
            .map(|ip| LanEndpoint {
                ip: ip.to_string(),
                port: RECEIVER_PORT,
            })
            .collect(),
        mdns_service: Some("vilsend._tcp.local".into()),
    })
}

/// Every private IPv4 a peer on the same network could dial, sorted and
/// deduplicated. Pure, so the selection rules are tested without real NICs.
fn lan_ipv4s(
    interfaces: impl IntoIterator<Item = (String, std::net::IpAddr)>,
) -> Vec<std::net::Ipv4Addr> {
    let mut ips: Vec<std::net::Ipv4Addr> = interfaces
        .into_iter()
        .filter(|(name, _)| !is_unsuitable_interface(name))
        .filter_map(|(_, address)| match address {
            std::net::IpAddr::V4(ip) if is_private_lan(ip) => Some(ip),
            _ => None,
        })
        .collect();

    ips.sort_unstable();
    ips.dedup();
    ips
}

fn is_unsuitable_interface(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    UNSUITABLE_INTERFACE_MARKERS
        .iter()
        .any(|marker| name.contains(marker))
}

/// RFC1918, plus the 100.64/10 carrier-grade NAT range overlays such as
/// Tailscale use. Public addresses are not offered as LAN endpoints.
fn is_private_lan(ip: std::net::Ipv4Addr) -> bool {
    if !is_dialable(ip) || ip.is_multicast() {
        return false;
    }

    let [first, second, _, _] = ip.octets();

    ip.is_private() || (first == 100 && (64..=127).contains(&second))
}

fn is_dialable(ip: std::net::Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
}

fn discover_lan_ipv4s() -> Vec<std::net::Ipv4Addr> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    let interfaces: Vec<(String, std::net::IpAddr)> = networks
        .iter()
        .flat_map(|(name, data)| {
            data.ip_networks()
                .iter()
                .map(move |network| (name.clone(), network.addr))
        })
        .collect();

    for (name, address) in &interfaces {
        if let std::net::IpAddr::V4(ip) = address {
            if is_private_lan(*ip) && !is_unsuitable_interface(name) {
                tracing::debug!(interface = %name, %ip, "lan probe: usable interface");
            }
        }
    }

    lan_ipv4s(interfaces)
}

/// The address of the interface the default route uses.
///
/// A UDP `connect` sends no packet; it only makes the kernel choose a source
/// address. Used to order, and if necessary replace, the enumerated
/// interfaces when none look suitable.
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
        let endpoints: Vec<&LanEndpoint> = manifest
            .lan
            .as_ref()
            .map(|lan| lan.endpoints.iter().collect())
            .unwrap_or_default();

        if endpoints.is_empty() {
            return None;
        }

        /*
         * Race every offered address. The receiver may advertise several
         * interfaces, and dialling them in sequence would cost a full
         * LAN_DIAL_TIMEOUT for each wrong one before the tunnel fallback.
         */
        let dials = endpoints.into_iter().map(|endpoint| {
            let ip = endpoint.ip.clone();
            let port = endpoint.port;

            Box::pin(async move {
                if lan_reachable(&ip, port).await {
                    Ok(SelectedTransport {
                        kind: TransportKind::Lan,
                        direct_endpoint: Some(format!("http://{ip}:{port}")),
                    })
                } else {
                    Err(())
                }
            })
        });

        if let Ok((selected, _)) = futures_util::future::select_ok(dials).await {
            return Some(selected);
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
    fn an_empty_manifest_keeps_every_capability_false() {
        let value = empty_manifest();

        assert_eq!(value["manifest_version"], 1);
        assert_eq!(value["capabilities"]["lan"], false);
        assert_eq!(value["capabilities"]["p2p"], false);
        assert_eq!(value["capabilities"]["ble"], false);
        assert!(value["capabilities"]["preferred_order"]
            .as_array()
            .expect("preferred_order is a list")
            .is_empty());
        assert!(value.get("lan").is_none(), "no LAN section without LAN");
    }

    #[test]
    fn a_probed_manifest_always_carries_capabilities() {
        let manifest = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(probe());
        let capabilities = manifest
            .capabilities
            .expect("capabilities are always present");

        assert!(!capabilities.p2p);
        assert!(!capabilities.ble);
        assert_eq!(
            capabilities.preferred_order.contains(&"lan".to_owned()),
            capabilities.lan,
            "LAN is preferred exactly when it is available"
        );
        assert_eq!(manifest.lan.is_some(), capabilities.lan);
    }

    #[test]
    fn only_private_non_virtual_ipv4s_are_advertised() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

        let v4 = |a, b, c, d| IpAddr::V4(Ipv4Addr::new(a, b, c, d));

        let interfaces = vec![
            ("Wi-Fi".to_owned(), v4(192, 168, 1, 20)),
            ("Ethernet".to_owned(), v4(10, 0, 0, 5)),
            // The same address twice: advertised once.
            ("Wi-Fi".to_owned(), v4(192, 168, 1, 20)),
            ("Loopback Pseudo-Interface 1".to_owned(), v4(127, 0, 0, 1)),
            ("vEthernet (Default Switch)".to_owned(), v4(172, 20, 0, 1)),
            ("VMware Network Adapter VMnet8".to_owned(), v4(192, 168, 56, 1)),
            // APIPA: no DHCP lease, nothing to reach.
            ("Wi-Fi".to_owned(), v4(169, 254, 10, 2)),
            // Public: not a LAN endpoint.
            ("Ethernet 2".to_owned(), v4(203, 0, 113, 9)),
            ("eth0".to_owned(), IpAddr::V6(Ipv6Addr::LOCALHOST)),
            // Overlay address a peer on the same tailnet can reach.
            ("tailscale0".to_owned(), v4(100, 101, 2, 3)),
        ];

        assert_eq!(
            lan_ipv4s(interfaces),
            vec![
                Ipv4Addr::new(10, 0, 0, 5),
                Ipv4Addr::new(100, 101, 2, 3),
                Ipv4Addr::new(192, 168, 1, 20),
            ]
        );
    }

    #[test]
    fn virtual_interface_names_are_rejected_case_insensitively() {
        for name in [
            "vEthernet (LAN)",
            "Docker0",
            "BR-abc123",
            "utun4",
            "Bluetooth Network Connection",
            "Loopback Pseudo-Interface 1",
        ] {
            assert!(is_unsuitable_interface(name), "{name} should be filtered");
        }

        for name in ["Wi-Fi", "Ethernet", "eth0", "wlan0", "en0", "tailscale0"] {
            assert!(!is_unsuitable_interface(name), "{name} should be kept");
        }
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
