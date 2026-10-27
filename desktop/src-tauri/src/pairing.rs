//! The pairing QR code (SPEC.md section 6.3): everything a phone needs to
//! trust this computer and reach it, as a `beam://pair` link.
//!
//! ```text
//! beam://pair?v=1&id=<16 hex>&key=<64 hex>&name=<percent-encoded>&addr=<ip:port,ip:port>
//! ```

use std::net::IpAddr;

use beam_core::LocalDevice;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    pub uri: String,
    /// The QR code as an SVG document.
    pub svg: String,
    pub device_id: String,
    pub addresses: Vec<String>,
}

pub fn pairing_info(me: &LocalDevice, port: u16) -> PairingInfo {
    let addresses = lan_addresses()
        .into_iter()
        .map(|ip| match ip {
            IpAddr::V4(v4) => format!("{v4}:{port}"),
            IpAddr::V6(v6) => format!("[{v6}]:{port}"),
        })
        .collect::<Vec<_>>();
    let uri = pairing_uri(me, &addresses);
    let svg = qrcode::QrCode::new(uri.as_bytes())
        .map(|code| {
            code.render::<qrcode::render::svg::Color>()
                .min_dimensions(256, 256)
                .quiet_zone(true)
                .build()
        })
        .unwrap_or_default();
    PairingInfo {
        uri,
        svg,
        device_id: me.device_id().to_string(),
        addresses,
    }
}

fn pairing_uri(me: &LocalDevice, addresses: &[String]) -> String {
    format!(
        "beam://pair?v=1&id={}&key={}&name={}&addr={}",
        hex::encode(me.device_id().0),
        hex::encode(me.identity.public_key()),
        percent_encode(&me.name),
        percent_encode(&addresses.join(",")),
    )
}

/// Non-loopback addresses a phone on the same network could use: IPv4
/// first, then global IPv6 (link-local IPv6 needs a scope id and is left out).
fn lan_addresses() -> Vec<IpAddr> {
    let mut ips: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|interface| interface.ip())
        .filter(|ip| match ip {
            IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_link_local(),
            IpAddr::V6(v6) => !v6.is_loopback() && v6.segments()[0] & 0xffc0 != 0xfe80,
        })
        .collect();
    ips.sort_by_key(|ip| (ip.is_ipv6(), *ip));
    ips.dedup();
    ips
}

/// Percent-encodes everything except RFC 3986 unreserved characters.
fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use beam_core::{DeviceType, Identity};

    #[test]
    fn uri_carries_id_key_name_and_addresses() {
        let me = LocalDevice {
            identity: Identity::generate(),
            name: "Kartikay's PC".into(),
            device_type: DeviceType::Laptop,
        };
        let uri = pairing_uri(&me, &["192.168.1.20:9000".into(), "10.0.0.5:9000".into()]);
        assert!(uri.starts_with("beam://pair?v=1&id="));
        assert!(uri.contains(&format!("&key={}", hex::encode(me.identity.public_key()))));
        assert!(uri.contains("&name=Kartikay%27s%20PC"));
        assert!(uri.contains("&addr=192.168.1.20%3A9000%2C10.0.0.5%3A9000"));
    }

    #[test]
    fn qr_code_renders() {
        let me = LocalDevice {
            identity: Identity::generate(),
            name: "PC".into(),
            device_type: DeviceType::Laptop,
        };
        let info = pairing_info(&me, 9000);
        assert!(info.svg.starts_with("<?xml") || info.svg.starts_with("<svg"));
        assert!(info.svg.contains("<svg"));
    }
}
