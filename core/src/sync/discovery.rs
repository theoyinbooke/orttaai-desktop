//! Finding Orttaai on a Mac on the home network: Bonjour `_orttaai._tcp`
//! (`docs/orttaai-link.md` §3).

use super::wire::{BONJOUR_TYPE, DEFAULT_PORT};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// A Mac offering Orttaai Link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredMac {
    pub mac_device_id: String,
    pub name: String,
    pub fingerprint: String,
    /// LAN addresses, then the `.local` name.
    pub hosts: Vec<String>,
    pub port: u16,
    /// Its Link window is open, so a code works now.
    pub linking: bool,
}

/// Looks for Macs for up to `wait`, returning early once one is found and a
/// moment has passed for others to answer.
pub fn discover(wait: Duration) -> Result<Vec<DiscoveredMac>, String> {
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let receiver = daemon.browse(BONJOUR_TYPE).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut found: Vec<DiscoveredMac> = Vec::new();
    let mut first_at: Option<Instant> = None;
    while started.elapsed() < wait {
        if first_at.is_some_and(|t| t.elapsed() > Duration::from_millis(1500)) {
            break;
        }
        let left = wait
            .saturating_sub(started.elapsed())
            .min(Duration::from_millis(300));
        let Ok(event) = receiver.recv_timeout(left) else {
            continue;
        };
        if let ServiceEvent::ServiceResolved(info) = event {
            let txt = |key: &str| info.get_property_val_str(key).map(str::to_string);
            let (Some(id), Some(fingerprint)) = (txt("id"), txt("fp")) else {
                continue;
            };
            let mut hosts: Vec<String> = txt("ips")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            for address in info.get_addresses() {
                let text = address.to_string();
                if address.is_ipv4() && !hosts.contains(&text) {
                    hosts.push(text);
                }
            }
            if let Some(local) = txt("host").filter(|h| !h.is_empty()) {
                hosts.push(local);
            }
            let instance = info
                .get_fullname()
                .split("._orttaai")
                .next()
                .unwrap_or("Mac")
                .replace("\\032", " ");
            let mac = DiscoveredMac {
                mac_device_id: id,
                name: instance,
                fingerprint: fingerprint.to_lowercase(),
                hosts,
                port: txt("port").and_then(|p| p.parse().ok()).unwrap_or_else(|| {
                    if info.get_port() > 0 {
                        info.get_port()
                    } else {
                        DEFAULT_PORT
                    }
                }),
                linking: txt("pair").as_deref() == Some("1"),
            };
            if let Some(existing) = found
                .iter_mut()
                .find(|m| m.mac_device_id == mac.mac_device_id)
            {
                *existing = mac;
            } else {
                found.push(mac);
            }
            first_at.get_or_insert_with(Instant::now);
        }
    }
    let _ = daemon.shutdown();
    Ok(found)
}
