//! The pairing QR code (SPEC.md section 6.3): a `beam://pair` link with this
//! computer's id, key, name and addresses, plus a one-time token (see
//! `beam_core::pairing`). A phone that scans it trusts this computer and,
//! by presenting the token, gets trusted back.

use std::net::{IpAddr, SocketAddr};

use beam_core::{LocalDevice, PairingCode, PairingWindow};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    pub uri: String,
    /// The QR code as an SVG document.
    pub svg: String,
    pub device_id: String,
    pub addresses: Vec<String>,
    /// How long the code stays valid.
    pub valid_for_secs: u64,
}

/// Opens a new pairing window (earlier codes stop working) and returns the
/// code to show.
pub fn pairing_info(me: &LocalDevice, port: u16, window: &PairingWindow) -> PairingInfo {
    let code = PairingCode {
        device_id: me.device_id(),
        public_key: *me.identity.public_key(),
        name: me.name.clone(),
        addresses: lan_addresses()
            .into_iter()
            .map(|ip| SocketAddr::new(ip, port))
            .collect(),
        token: window.open(),
    };
    let uri = code.to_uri();
    let svg = qrcode::QrCode::new(uri.as_bytes())
        .map(|qr| {
            qr.render::<qrcode::render::svg::Color>()
                .min_dimensions(256, 256)
                .quiet_zone(true)
                .build()
        })
        .unwrap_or_default();
    PairingInfo {
        uri,
        svg,
        device_id: me.device_id().to_string(),
        addresses: code.addresses.iter().map(|a| a.to_string()).collect(),
        valid_for_secs: beam_core::pairing::PAIRING_CODE_LIFETIME.as_secs(),
    }
}

/// Non-loopback addresses a phone on the same network could use: IPv4
/// first, then global IPv6 (link-local IPv6 needs a scope id and is left out).
pub(crate) fn lan_addresses() -> Vec<IpAddr> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use beam_core::{DeviceType, Identity};

    #[test]
    fn code_parses_back_and_each_view_gets_a_new_token() {
        let me = LocalDevice {
            identity: Identity::generate(),
            name: "Kartikay's PC".into(),
            device_type: DeviceType::Laptop,
        };
        let window = PairingWindow::default();
        let first = pairing_info(&me, 9000, &window);
        let second = pairing_info(&me, 9000, &window);
        let parsed = PairingCode::parse(&first.uri).unwrap();
        assert_eq!(parsed.device_id, me.device_id());
        assert_eq!(parsed.name, "Kartikay's PC");
        assert_ne!(first.uri, second.uri, "a fresh one-time token each time");
        assert!(first.svg.contains("<svg"));
    }
}
