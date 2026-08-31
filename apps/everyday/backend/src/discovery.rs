//! Authenticated offline-LAN peer discovery. Discovery only finds transport
//! endpoints; signed journal trust remains a separate explicit decision.

use crate::{sync, AppState};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;

const DOMAIN: &str = "everyday/lan-discovery/v1";
const VERSION: u8 = 1;
const MAX_CLOCK_SKEW_SECONDS: i64 = 120;
const MAX_PACKET_BYTES: usize = 2_048;

#[derive(Default)]
struct ReplayCache(HashMap<String, i64>);

impl ReplayCache {
    fn accept(&mut self, announcement: &Announcement, current: i64) -> bool {
        self.0
            .retain(|_, seen_at| current - *seen_at <= MAX_CLOCK_SKEW_SECONDS);
        let key = format!("{}:{}", announcement.node_id, announcement.nonce);
        if self.0.contains_key(&key) {
            return false;
        }
        self.0.insert(key, current);
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Announcement {
    v: u8,
    node_id: String,
    url: String,
    at: i64,
    nonce: String,
    mac: String,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}

fn payload(node_id: &str, url: &str, at: i64, nonce: &str) -> String {
    format!("{DOMAIN}\n{VERSION}\n{node_id}\n{url}\n{at}\n{nonce}")
}

fn mac(token: &str, bytes: &[u8]) -> Vec<u8> {
    let mut hmac = Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC accepts any key");
    hmac.update(bytes);
    hmac.finalize().into_bytes().to_vec()
}

fn make_announcement(token: &str, node_id: &str, url: &str, at: i64) -> Announcement {
    let mut random = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut random);
    let nonce = hex::encode(random);
    let signed = payload(node_id, url, at, &nonce);
    Announcement {
        v: VERSION,
        node_id: node_id.to_owned(),
        url: url.to_owned(),
        at,
        nonce,
        mac: hex::encode(mac(token, signed.as_bytes())),
    }
}

fn local_transport_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || url.port().is_none()
        || !matches!(url.scheme(), "http" | "https")
    {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let Ok(ip) = host.parse::<IpAddr>() else {
        return false;
    };
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
        IpAddr::V6(ip) => {
            ip.is_loopback() || ip.is_unicast_link_local() || (ip.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

fn verify_announcement(
    bytes: &[u8],
    token: &str,
    local_node_id: &str,
    at: i64,
) -> anyhow::Result<Announcement> {
    if bytes.len() > MAX_PACKET_BYTES {
        anyhow::bail!("discovery packet is too large");
    }
    let announcement: Announcement = serde_json::from_slice(bytes)?;
    if announcement.v != VERSION
        || announcement.node_id.is_empty()
        || announcement.node_id == local_node_id
        || announcement.nonce.len() != 32
        || !announcement
            .nonce
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || (announcement.at - at).abs() > MAX_CLOCK_SKEW_SECONDS
        || !local_transport_url(&announcement.url)
    {
        anyhow::bail!("invalid or stale discovery announcement");
    }
    let supplied = hex::decode(&announcement.mac)?;
    let expected = payload(
        &announcement.node_id,
        &announcement.url,
        announcement.at,
        &announcement.nonce,
    );
    Hmac::<Sha256>::new_from_slice(token.as_bytes())?
        .chain_update(expected.as_bytes())
        .verify_slice(&supplied)
        .map_err(|_| anyhow::anyhow!("invalid discovery authentication"))?;
    Ok(announcement)
}

pub async fn run(state: Arc<AppState>, token: String, bind: String, target: String) {
    let socket = match UdpSocket::bind(&bind).await {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("LAN discovery не запущен ({bind}): {error}");
            return;
        }
    };
    if let Err(error) = socket.set_broadcast(true) {
        eprintln!("LAN discovery broadcast недоступен: {error}");
        return;
    }
    let (node_id, url) = {
        let db = state.db.lock();
        (sync::ensure_node(&db).0, sync::guess_lan_base())
    };
    if !local_transport_url(&url) {
        eprintln!(
            "LAN discovery выключен: MESHKEEPER_ADVERTISE_URL не является локальным IP endpoint"
        );
        return;
    }
    eprintln!("LAN discovery {bind} → {target}, объявляю {url}");
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    let mut buffer = [0_u8; MAX_PACKET_BYTES + 1];
    let mut replay_cache = ReplayCache::default();
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let packet = make_announcement(&token, &node_id, &url, now());
                if let Ok(bytes) = serde_json::to_vec(&packet) {
                    let _ = socket.send_to(&bytes, &target).await;
                }
            }
            received = socket.recv_from(&mut buffer) => {
                let Ok((length, _source)) = received else { continue };
                let Ok(peer) = verify_announcement(&buffer[..length], &token, &node_id, now()) else { continue };
                if !replay_cache.accept(&peer, now()) { continue; }
                let added = {
                    let db = state.db.lock();
                    sync::add_peer(&db, &peer.url, Some("LAN auto-discovery"), Some(&peer.node_id))
                };
                if added.get("added").and_then(serde_json::Value::as_bool) == Some(true) {
                    sync::request_sync_now();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "test-discovery-token-that-is-long-enough";

    #[test]
    fn authentic_announcement_round_trips_and_tampering_is_rejected() {
        let packet = make_announcement(TOKEN, "node-a", "http://192.168.4.2:8766", 1_000);
        let bytes = serde_json::to_vec(&packet).unwrap();
        assert_eq!(
            verify_announcement(&bytes, TOKEN, "node-b", 1_030)
                .unwrap()
                .node_id,
            "node-a"
        );

        let mut forged: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        forged["url"] = serde_json::json!("http://192.168.4.99:8766");
        assert!(verify_announcement(
            &serde_json::to_vec(&forged).unwrap(),
            TOKEN,
            "node-b",
            1_030
        )
        .is_err());
        assert!(verify_announcement(
            &bytes,
            "wrong-token-that-is-also-long-enough",
            "node-b",
            1_030
        )
        .is_err());
    }

    #[test]
    fn discovery_rejects_replay_self_and_non_local_targets() {
        let packet = make_announcement(TOKEN, "node-a", "http://10.20.30.40:8766", 1_000);
        let bytes = serde_json::to_vec(&packet).unwrap();
        let mut replay = ReplayCache::default();
        assert!(replay.accept(&packet, 1_000));
        assert!(!replay.accept(&packet, 1_001));
        assert!(verify_announcement(&bytes, TOKEN, "node-b", 1_121).is_err());
        assert!(verify_announcement(&bytes, TOKEN, "node-a", 1_000).is_err());
        for url in [
            "http://example.com:8766",
            "http://8.8.8.8:8766",
            "http://127.0.0.1",
            "file:///tmp/socket",
        ] {
            let value = make_announcement(TOKEN, "node-a", url, 1_000);
            assert!(
                verify_announcement(&serde_json::to_vec(&value).unwrap(), TOKEN, "node-b", 1_000)
                    .is_err(),
                "accepted {url}"
            );
        }
    }
}
