//! Which network paths a transfer had, written when it starts and kept current
//! while it runs.
//!
//! One file per transfer per side, `<transfer-id>.network.json` beside the final
//! report. The sender fills it from the `START_TRANSFER` command the backend
//! sent — the tunnel endpoint plus the receiver's relayed manifest. The
//! receiver fills it from the manifest it advertised when it accepted. Both are
//! rewritten at the same cadence as the sender's checkpoint, so the file tracks
//! the transfer instead of describing it after the fact.

use serde::Serialize;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::transfer::report::Role;
use crate::transfer::transport_manifest::{transport_label, TransportManifest};
use vilsend_core::TransportKind;

/// The report format this build writes.
pub const NETWORK_REPORT_SCHEMA_VERSION: u32 = 1;

/// How often a running transfer refreshes the file.
pub const NETWORK_REPORT_INTERVAL: Duration = Duration::from_secs(2);

/// One path offer, as the start request described it.
#[derive(Debug, Clone, Serialize)]
pub struct NetworkPath {
    /// The label the UI uses: `LAN`, `P2P` or `Tunnel`.
    pub kind: &'static str,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Why the path is not usable, when it is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkReport {
    pub schema_version: u32,
    pub role: Role,
    pub transfer_id: String,
    pub started_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    pub status: String,
    pub paths: Vec<NetworkPath>,
    /// The path this side chose or believes is in use, when it knows.
    pub selected: Option<String>,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub completed_chunks: u64,
    pub total_chunks: u64,
}

impl NetworkReport {
    pub fn new(
        role: Role,
        transfer_id: &str,
        paths: Vec<NetworkPath>,
        selected: Option<String>,
    ) -> Self {
        let now = now_ms();

        Self {
            schema_version: NETWORK_REPORT_SCHEMA_VERSION,
            role,
            transfer_id: transfer_id.to_owned(),
            started_at_unix_ms: now,
            updated_at_unix_ms: now,
            status: "QUEUED".into(),
            paths,
            selected,
            completed_bytes: 0,
            total_bytes: 0,
            completed_chunks: 0,
            total_chunks: 0,
        }
    }

    /// A report for a transfer restored from disk, where only the endpoint
    /// survived. The manifest-derived detail is gone, and the file says so
    /// rather than guessing from what this machine can see now.
    pub fn restored(role: Role, transfer_id: &str, endpoint: &str) -> Self {
        let mut report = Self::new(role, transfer_id, paths(None, Some(endpoint)), None);

        for path in report.paths.iter_mut().filter(|path| {
            path.kind == transport_label(TransportKind::Lan)
                || path.kind == transport_label(TransportKind::P2p)
        }) {
            path.reason = Some("the start request is not persisted across a restart".to_owned());
        }

        report
    }

    pub fn update(
        &mut self,
        status: &str,
        completed_bytes: u64,
        total_bytes: u64,
        completed_chunks: u64,
        total_chunks: u64,
    ) {
        self.updated_at_unix_ms = now_ms();
        self.status = status.to_owned();
        self.completed_bytes = completed_bytes;
        self.total_bytes = total_bytes;
        self.completed_chunks = completed_chunks;
        self.total_chunks = total_chunks;
    }
}

/// The paths a manifest offers, plus the tunnel the control plane named.
///
/// `tunnel_endpoint` is `None` on the receiver, which is never told which route
/// the sender was given.
pub fn paths(
    manifest: Option<&TransportManifest>,
    tunnel_endpoint: Option<&str>,
) -> Vec<NetworkPath> {
    let capabilities = manifest.and_then(|manifest| manifest.capabilities.as_ref());

    let lan = manifest
        .and_then(|manifest| manifest.lan.as_ref())
        .and_then(|lan| lan.endpoints.first())
        .map(|endpoint| format!("http://{}:{}", endpoint.ip, endpoint.port));

    let lan_available = capabilities.is_some_and(|capabilities| capabilities.lan) && lan.is_some();

    let lan_reason = if lan_available {
        None
    } else if manifest.is_none() {
        Some("no transport manifest accompanied the start request".to_owned())
    } else if !capabilities.is_some_and(|capabilities| capabilities.lan) {
        Some("the peer did not advertise LAN".to_owned())
    } else {
        Some("the manifest advertised LAN but named no endpoint".to_owned())
    };

    let p2p_advertised = capabilities.is_some_and(|capabilities| capabilities.p2p);

    vec![
        NetworkPath {
            kind: transport_label(TransportKind::Lan),
            available: lan_available,
            endpoint: lan,
            reason: lan_reason,
        },
        NetworkPath {
            kind: transport_label(TransportKind::P2p),
            available: p2p_advertised,
            endpoint: None,
            reason: Some(if p2p_advertised {
                "advertised by the peer; this build dials only LAN and the tunnel".to_owned()
            } else {
                "not advertised by the peer".to_owned()
            }),
        },
        NetworkPath {
            kind: transport_label(TransportKind::Tunnel),
            available: true,
            endpoint: tunnel_endpoint.map(str::to_owned),
            reason: tunnel_endpoint
                .is_none()
                .then(|| "the control-plane route is not known on this side".to_owned()),
        },
    ]
}

/// Where one transfer's network report lives, beside its final report.
pub fn path(reports_dir: &Path, transfer_id: &str) -> PathBuf {
    crate::transfer::report::report_path(reports_dir, transfer_id).with_extension("network.json")
}

/// Write the report atomically. Best effort: a transfer never fails over it.
pub async fn write(reports_dir: &Path, report: &NetworkReport) -> io::Result<()> {
    tokio::fs::create_dir_all(reports_dir).await?;

    let json = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;

    crate::transfer::atomic_file::write_atomic(&path(reports_dir, &report.transfer_id), &json).await
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(value: serde_json::Value) -> TransportManifest {
        serde_json::from_value(value).expect("test manifest parses")
    }

    #[test]
    fn without_a_manifest_only_the_tunnel_is_available() {
        let paths = paths(None, Some("https://relay.example/t/1"));

        assert_eq!(paths.len(), 3);
        assert!(!paths[0].available);
        assert!(!paths[1].available);
        assert!(paths[2].available);
        assert_eq!(paths[2].endpoint.as_deref(), Some("https://relay.example/t/1"));
    }

    #[test]
    fn a_lan_manifest_names_its_endpoint() {
        let manifest = manifest(serde_json::json!({
            "capabilities": { "lan": true, "p2p": false },
            "lan": { "endpoints": [{ "ip": "192.168.1.5", "port": 9443 }] }
        }));

        let paths = paths(Some(&manifest), None);

        assert!(paths[0].available);
        assert_eq!(paths[0].endpoint.as_deref(), Some("http://192.168.1.5:9443"));
        assert!(!paths[1].available);
    }

    #[test]
    fn a_p2p_advertisement_is_reported_but_not_dialled() {
        let manifest = manifest(serde_json::json!({
            "capabilities": { "lan": false, "p2p": true }
        }));

        let paths = paths(Some(&manifest), None);

        assert!(paths[1].available);
        assert!(paths[1].reason.as_deref().unwrap().contains("dials only LAN"));
    }

    #[test]
    fn the_file_sits_beside_the_final_report() {
        let dir = Path::new("reports");

        assert_eq!(
            path(dir, "t/../1"),
            dir.join("t____1.network.json"),
            "the id is sanitised to one file name"
        );
    }
}
