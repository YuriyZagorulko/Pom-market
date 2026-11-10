//! Client IP extraction with optional trusted reverse proxies.

use std::net::{IpAddr, SocketAddr};

use axum::http::HeaderMap;
use ipnet::IpNet;

const MAX_FORWARDED_LEN: usize = 512;

#[derive(Clone, Debug, Default)]
pub struct TrustedProxies {
    nets: Vec<IpNet>,
}

#[derive(Debug, Default)]
pub struct TrustedProxiesParse {
    pub nets: Vec<IpNet>,
    pub errors: Vec<String>,
}

impl TrustedProxies {
    /// Parse `TRUSTED_PROXIES`. Invalid tokens are collected in `errors`; wildcards are rejected.
    pub fn parse_config(raw: &str) -> TrustedProxiesParse {
        let mut out = TrustedProxiesParse::default();
        for part in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if part == "*" || part == "0.0.0.0/0" || part == "::/0" {
                out.errors
                    .push(format!("TRUSTED_PROXIES rejects wildcard entry: {part}"));
                continue;
            }
            match part.parse::<IpNet>() {
                Ok(net) => {
                    if net.prefix_len() == 0 {
                        out.errors.push(format!(
                            "TRUSTED_PROXIES rejects full-internet CIDR: {part}"
                        ));
                        continue;
                    }
                    out.nets.push(net);
                }
                Err(_) => {
                    if let Ok(addr) = part.parse::<IpAddr>() {
                        let suffix = if addr.is_ipv4() { "/32" } else { "/128" };
                        if let Ok(net) = format!("{addr}{suffix}").parse::<IpNet>() {
                            out.nets.push(net);
                            continue;
                        }
                    }
                    out.errors
                        .push(format!("TRUSTED_PROXIES has invalid entry: {part}"));
                }
            }
        }
        out
    }

    pub fn from_parsed(parsed: TrustedProxiesParse) -> Self {
        Self { nets: parsed.nets }
    }

    pub fn from_config(raw: &str) -> Self {
        Self::from_parsed(Self::parse_config(raw))
    }

    pub fn is_empty(&self) -> bool {
        self.nets.is_empty()
    }

    pub fn trusts(&self, addr: IpAddr) -> bool {
        self.nets
            .iter()
            .any(|net| net.contains(&normalize_ip(addr)))
    }

    pub fn trusts_peer(&self, peer: SocketAddr) -> bool {
        self.trusts(peer.ip())
    }
}

/// Map `::ffff:a.b.c.d` to IPv4 so trust checks and rate-limit keys stay consistent.
pub fn normalize_ip(addr: IpAddr) -> IpAddr {
    match addr {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        other => other,
    }
}

/// Resolve the client IP for rate limiting and audit.
///
/// - If the TCP peer is not a trusted proxy, ignore forwarded headers and use the peer address.
/// - If the peer is trusted, walk `X-Forwarded-For` from right to left and return the first
///   untrusted hop. Malformed or oversized headers fall back to the peer address.
pub fn resolve_client_ip(
    peer: SocketAddr,
    headers: &HeaderMap,
    trusted: &TrustedProxies,
) -> String {
    let peer_ip = normalize_ip(peer.ip());
    if !trusted.trusts(peer_ip) {
        return peer_ip.to_string();
    }

    let Some(raw) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) else {
        return peer_ip.to_string();
    };

    if raw.len() > MAX_FORWARDED_LEN {
        return peer_ip.to_string();
    }

    let hops: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .collect();

    for hop in hops.iter().rev() {
        let Ok(addr) = hop.parse::<IpAddr>() else {
            break;
        };
        let addr = normalize_ip(addr);
        if trusted.trusts(addr) {
            continue;
        }
        return addr.to_string();
    }

    peer_ip.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trusted_local() -> TrustedProxies {
        TrustedProxies::from_config("127.0.0.0/8")
    }

    #[test]
    fn ignores_xff_from_untrusted_peer() {
        let peer: SocketAddr = "203.0.113.1:1234".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.10".parse().unwrap());
        let ip = resolve_client_ip(peer, &headers, &trusted_local());
        assert_eq!(ip, "203.0.113.1");
    }

    #[test]
    fn uses_client_from_xff_chain_behind_trusted_peer() {
        let peer: SocketAddr = "127.0.0.1:4321".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.5, 127.0.0.1".parse().unwrap());
        let ip = resolve_client_ip(peer, &headers, &trusted_local());
        assert_eq!(ip, "203.0.113.5");
    }

    #[test]
    fn skips_trusted_hops_in_chain() {
        let trusted = TrustedProxies::from_config("127.0.0.0/8, 192.0.2.0/24");
        let peer: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "198.51.100.7, 192.0.2.1, 127.0.0.1".parse().unwrap(),
        );
        let ip = resolve_client_ip(peer, &headers, &trusted);
        assert_eq!(ip, "198.51.100.7");
    }

    #[test]
    fn left_spoof_in_chain_is_ignored_when_hops_are_trusted() {
        let trusted = TrustedProxies::from_config("127.0.0.0/8");
        let peer: SocketAddr = "127.0.0.1:9".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "198.51.100.1, 198.51.100.2, 127.0.0.1".parse().unwrap(),
        );
        let ip = resolve_client_ip(peer, &headers, &trusted);
        assert_eq!(ip, "198.51.100.2");
    }

    #[test]
    fn ipv6_client_behind_trusted_peer() {
        let trusted = TrustedProxies::from_config("127.0.0.0/8");
        let peer: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "2001:db8::5, 127.0.0.1".parse().unwrap());
        let ip = resolve_client_ip(peer, &headers, &trusted);
        assert_eq!(ip, "2001:db8::5");
    }

    #[test]
    fn ipv4_mapped_peer_matches_ipv4_trust() {
        let trusted = TrustedProxies::from_config("203.0.113.0/24");
        let peer: SocketAddr = "[::ffff:203.0.113.10]:4321".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.3".parse().unwrap());
        let ip = resolve_client_ip(peer, &headers, &trusted);
        assert_eq!(ip, "198.51.100.3");
    }

    #[test]
    fn mapped_client_hop_is_normalized() {
        let trusted = TrustedProxies::from_config("127.0.0.0/8");
        let peer: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "::ffff:203.0.113.8, 127.0.0.1".parse().unwrap(),
        );
        let ip = resolve_client_ip(peer, &headers, &trusted);
        assert_eq!(ip, "203.0.113.8");
    }

    #[test]
    fn malformed_xff_falls_back_to_peer() {
        let trusted = TrustedProxies::from_config("127.0.0.0/8");
        let peer: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "not-an-ip, 127.0.0.1".parse().unwrap());
        let ip = resolve_client_ip(peer, &headers, &trusted);
        assert_eq!(ip, "127.0.0.1");
    }

    #[test]
    fn oversized_forwarded_header_falls_back_to_peer() {
        let peer: SocketAddr = "127.0.0.1:9".parse().unwrap();
        let mut headers = HeaderMap::new();
        let long = "1".repeat(MAX_FORWARDED_LEN + 1);
        headers.insert("x-forwarded-for", long.parse().unwrap());
        let ip = resolve_client_ip(peer, &headers, &trusted_local());
        assert_eq!(ip, "127.0.0.1");
    }

    #[test]
    fn wildcard_proxy_list_is_rejected() {
        let parsed = TrustedProxies::parse_config("*, 127.0.0.1");
        assert!(!parsed.errors.is_empty());
        assert!(parsed
            .nets
            .iter()
            .any(|n| n.to_string().contains("127.0.0.1")));
    }

    #[test]
    fn invalid_entry_is_reported() {
        let parsed = TrustedProxies::parse_config("not-a-CIDR, 10.0.0.0/8");
        assert_eq!(parsed.errors.len(), 1);
        assert!(parsed.errors[0].contains("not-a-CIDR"));
    }
}
