// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Shared middleware utilities.
//! [`get_client_ip`] is the single source of truth for client IP extraction:
//! proxy headers are trusted only when the direct peer is in `trusted_proxies`.

use crate::server::config::tls::PeerAddr;
use axum::body::Body;
use axum::http::Request;
use std::net::{IpAddr, SocketAddr};

/// Extract the originating client IP from a request.
///
/// `X-Forwarded-For` (first hop) then `X-Real-IP` are trusted only when the
/// direct peer is in `trusted_proxies`; empty slice disables them (default).
pub fn get_client_ip(req: &Request<Body>, trusted_proxies: &[IpAddr]) -> Option<String> {
    // 生产 DualListener 注入 ConnectInfo<PeerAddr>，axum 内置 TcpListener 注入
    // ConnectInfo<SocketAddr>；两者与裸 SocketAddr 都须识别，否则退化为共享单桶。
    let connection_ip = req
        .extensions()
        .get::<axum::extract::connect_info::ConnectInfo<PeerAddr>>()
        .map(|info| info.0 .0.ip())
        .or_else(|| {
            req.extensions()
                .get::<axum::extract::connect_info::ConnectInfo<SocketAddr>>()
                .map(|info| info.0.ip())
        })
        .or_else(|| req.extensions().get::<SocketAddr>().map(|addr| addr.ip()));

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
    use axum::http::Request;
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
    fn reads_connectinfo_extension_injected_by_axum_serve() {
        // axum 内置 TcpListener 注入 ConnectInfo<SocketAddr>；生产见下一个测试。
        let mut req = Request::builder().uri("/").body(Body::empty()).unwrap();
        req.extensions_mut()
            .insert(axum::extract::connect_info::ConnectInfo(SocketAddr::from(
                (Ipv4Addr::new(203, 0, 113, 7), 9999),
            )));
        assert_eq!(get_client_ip(&req, &[]).as_deref(), Some("203.0.113.7"));
    }

    #[test]
    fn reads_peeraddr_connectinfo_injected_by_dual_listener() {
        // 生产 DualListener 注入 ConnectInfo<PeerAddr>，未识别则 per-IP
        // 限流/认证失败计数全部退化为 anonymous 单桶。
        let mut req = Request::builder().uri("/").body(Body::empty()).unwrap();
        req.extensions_mut()
            .insert(axum::extract::connect_info::ConnectInfo(PeerAddr(
                SocketAddr::from((Ipv4Addr::new(203, 0, 113, 9), 9999)),
            )));
        assert_eq!(get_client_ip(&req, &[]).as_deref(), Some("203.0.113.9"));

        // 可信代理下 PeerAddr 同样参与判定
        let trusted: Vec<IpAddr> = vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))];
        let mut req = Request::builder().uri("/").body(Body::empty()).unwrap();
        req.extensions_mut()
            .insert(axum::extract::connect_info::ConnectInfo(PeerAddr(
                SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 443)),
            )));
        req.headers_mut()
            .insert("x-forwarded-for", "198.51.100.23".parse().unwrap());
        assert_eq!(
            get_client_ip(&req, &trusted).as_deref(),
            Some("198.51.100.23")
        );
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
