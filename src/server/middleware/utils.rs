// Copyright © 2026 Kirky.X
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Shared middleware utilities.
//!
//! Phase 9 T043 (HIGH H3 / LOW L3) — single source of truth for
//! `get_client_ip`. Previously this logic was duplicated in
//! `api_key_auth.rs`, `audit/middleware.rs`, `rate_limit/middleware.rs`,
//! and a dead-code copy in this file, with **inconsistent security
//! policies**: only `audit` and `rate_limit` checked `trusted_proxies`
//! before trusting `X-Forwarded-For`; `api_key_auth.rs` blindly trusted
//! the header, allowing an attacker to spoof IPs and bypass
//! IP-based auth-failure rate limiting (diting H3 / tiangang CWE-290).
//!
//! All four copies now delegate to [`get_client_ip`] below, which
//! enforces the `trusted_proxies` check uniformly.

use sdforge::axum::body::Body;
use sdforge::axum::http::Request;
use std::net::{IpAddr, SocketAddr};

/// Extract the originating client IP from a request.
///
/// Strategy (in order):
/// 1. If the direct peer (TCP connection) IP is in `trusted_proxies`,
///    trust `X-Forwarded-For` (first hop) then `X-Real-IP`.
/// 2. Otherwise, return the direct peer IP.
/// 3. If neither is available, return `None`.
///
/// # Security
///
/// `X-Forwarded-For` and `X-Real-IP` are **only** consulted when the
/// peer is a configured trusted proxy. This prevents clients from
/// spoofing their IP via a forged header to bypass IP-based rate
/// limiting or auth-failure tracking. An empty `trusted_proxies` slice
/// disables header-based IP discovery entirely (default).
pub fn get_client_ip(req: &Request<Body>, trusted_proxies: &[IpAddr]) -> Option<String> {
    let connection_ip = req.extensions().get::<SocketAddr>().map(|addr| addr.ip());

    if let Some(conn_ip) = connection_ip {
        if trusted_proxies.contains(&conn_ip) {
            if let Some(client_ip) = parse_xff(req) {
                return Some(client_ip);
            }
            if let Some(client_ip) = parse_xri(req) {
                return Some(client_ip);
            }
        }
    }

    connection_ip.map(|ip| ip.to_string())
}

fn parse_xff(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn parse_xri(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get("x-real-ip")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdforge::axum::http::Request;
    use std::net::{Ipv4Addr, SocketAddr};

    fn make_request(
        peer: Option<SocketAddr>,
        xff: Option<&str>,
        xri: Option<&str>,
    ) -> Request<Body> {
        let mut builder = Request::builder().uri("/").body(Body::empty()).unwrap();
        if let Some(addr) = peer {
            builder.extensions_mut().insert(addr);
        }
        if let Some(v) = xff {
            builder
                .headers_mut()
                .insert("x-forwarded-for", v.parse().unwrap());
        }
        if let Some(v) = xri {
            builder
                .headers_mut()
                .insert("x-real-ip", v.parse().unwrap());
        }
        builder
    }

    #[test]
    fn returns_direct_connection_ip_when_no_trusted_proxies() {
        let req = make_request(
            Some(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 8080))),
            Some("1.2.3.4"),
            None,
        );
        let ip = get_client_ip(&req, &[]).unwrap();
        assert_eq!(ip, "10.0.0.1");
    }

    #[test]
    fn trusts_xff_only_from_trusted_proxy() {
        let trusted: Vec<IpAddr> = vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))];
        let req = make_request(
            Some(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 8080))),
            Some("203.0.113.5, 10.0.0.1"),
            None,
        );
        let ip = get_client_ip(&req, &trusted).unwrap();
        assert_eq!(ip, "203.0.113.5");
    }

    #[test]
    fn ignores_xff_from_untrusted_peer() {
        let trusted: Vec<IpAddr> = vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))];
        let req = make_request(
            Some(SocketAddr::from((Ipv4Addr::new(192, 168, 0, 1), 8080))),
            Some("203.0.113.5"),
            None,
        );
        let ip = get_client_ip(&req, &trusted).unwrap();
        assert_eq!(ip, "192.168.0.1");
    }

    #[test]
    fn falls_back_to_xri_when_xff_absent() {
        let trusted: Vec<IpAddr> = vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))];
        let req = make_request(
            Some(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 8080))),
            None,
            Some("203.0.113.7"),
        );
        let ip = get_client_ip(&req, &trusted).unwrap();
        assert_eq!(ip, "203.0.113.7");
    }

    #[test]
    fn returns_none_when_no_ip_available() {
        let req = make_request(None, Some("1.2.3.4"), None);
        assert!(get_client_ip(&req, &[]).is_none());
    }
}
