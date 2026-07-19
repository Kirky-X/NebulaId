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

//! Locale negotiation middleware (Phase 8 T040).
//!
//! Parses the `Accept-Language` HTTP header per RFC 7231 §5.3.5,
//! picks the best supported locale (`en` or `zh-CN`), and injects
//! `Extension<Locale>` into the request for downstream handlers.
//!
//! Negotiation rules:
//! 1. Parse each `<language-tag>[;q=<weight>]` entry.
//! 2. Drop entries with `q=0` (explicitly not accepted) or malformed q-value
//!    (per RFC 7231 §5.3.1, the entire entry becomes invalid).
//! 3. Sort by descending q-value; ties keep header order (stable sort).
//! 4. For each candidate, try exact match → prefix match (e.g. `zh` → `zh-CN`).
//!    Wildcard `*` is treated as no concrete match and falls through to default.
//! 5. If no match, fall back to the default locale (`en`).
//!
//! Missing or malformed `Accept-Language` header also falls back to `en`.
//!
//! # Security
//!
//! `Locale` is **user-controlled** (derived from the `Accept-Language` header)
//! and MUST NOT be used for authorization, authentication, or any security
//! decision. It is intended solely for content negotiation (e.g. translating
//! error messages). Treating locale as a trust boundary will introduce
//! header-forging vulnerabilities.

use axum::body::Body;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::fmt;

/// Supported locales for NebulaID.
///
/// Variants match the top-level keys in `locales/*.yml`.
///
/// # Security
///
/// `Locale` is derived from user input (`Accept-Language` header) and is
/// forgeable. Do NOT use it for authorization decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Locale {
    /// English (default).
    En,
    /// Simplified Chinese.
    ZhCn,
}

impl Locale {
    /// Default locale when `Accept-Language` is missing or no match.
    pub const DEFAULT: Self = Locale::En;

    /// Return the string identifier used in `locales/*.yml` and
    /// `rust_i18n::set_locale`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::ZhCn => "zh-CN",
        }
    }

    /// Parse a language tag into a `Locale`.
    ///
    /// Matching is case-insensitive on the tag. `zh`, `zh-CN`, `zh-Hans`,
    /// and `zh-Hans-CN` all map to `ZhCn`. `en`, `en-US`, `en-GB` map to `En`.
    /// Unknown tags return `None`.
    pub fn from_tag(tag: &str) -> Option<Self> {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            return None;
        }
        // Exact match (case-insensitive, no allocation)
        if trimmed.eq_ignore_ascii_case("en") {
            return Some(Locale::En);
        }
        if trimmed.eq_ignore_ascii_case("zh-CN")
            || trimmed.eq_ignore_ascii_case("zh-Hans-CN")
            || trimmed.eq_ignore_ascii_case("zh-Hans")
        {
            return Some(Locale::ZhCn);
        }
        // Prefix match on the primary subtag
        let primary = trimmed.split('-').next().unwrap_or(trimmed);
        if primary.eq_ignore_ascii_case("en") {
            Some(Locale::En)
        } else if primary.eq_ignore_ascii_case("zh") {
            Some(Locale::ZhCn)
        } else {
            None
        }
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Axum middleware: negotiate locale from `Accept-Language` and inject
/// `Extension<Locale>` into the request.
pub async fn locale_middleware(mut req: Request<Body>, next: Next) -> Response {
    let locale = negotiate_locale(req.headers().get(axum::http::header::ACCEPT_LANGUAGE));
    req.extensions_mut().insert(locale);
    next.run(req).await
}

/// Negotiate the best `Locale` from an `Accept-Language` header value.
///
/// Accepts `None` (header missing) — returns `Locale::DEFAULT`.
pub fn negotiate_locale(header: Option<&axum::http::HeaderValue>) -> Locale {
    let Some(value) = header else {
        return Locale::DEFAULT;
    };
    let Ok(s) = value.to_str() else {
        return Locale::DEFAULT;
    };
    negotiate_locale_str(s)
}

/// Parse and negotiate locale from the raw `Accept-Language` string.
pub fn negotiate_locale_str(accept_language: &str) -> Locale {
    let candidates = parse_accept_language(accept_language);
    for candidate in candidates {
        // Skip wildcard — it never matches a concrete Locale
        if candidate.tag == "*" {
            continue;
        }
        if let Some(locale) = Locale::from_tag(&candidate.tag) {
            return locale;
        }
    }
    Locale::DEFAULT
}

/// A single entry in the `Accept-Language` header.
#[derive(Debug, PartialEq)]
struct Candidate {
    tag: String,
    quality: f32,
}

/// Maximum accepted `Accept-Language` header length (bytes).
///
/// Defends against DoS via pathologically long headers. Typical browser
/// headers are < 100 bytes; 4 KiB is a generous upper bound (hyper's
/// default MaxRequestHeaderSize is much larger, so we cap here).
const MAX_ACCEPT_LANGUAGE_LEN: usize = 4096;

/// Parse `Accept-Language` into a list of candidates sorted by descending
/// q-value (stable on ties to preserve header order).
///
/// Entries with `q=0` are dropped (explicitly not accepted). Entries with
/// malformed q-values are also dropped (per RFC 7231 §5.3.1, the entire
/// entry becomes invalid). Wildcard `*` is kept for explicit fallthrough
/// handling but never matches a concrete `Locale`.
///
/// Returns an empty `Vec` if the header is empty, too long, or contains
/// no valid entries.
fn parse_accept_language(header: &str) -> Vec<Candidate> {
    if header.trim().is_empty() {
        return Vec::new();
    }
    if header.len() > MAX_ACCEPT_LANGUAGE_LEN {
        return Vec::new();
    }
    // Pre-allocate based on comma count to avoid reallocation
    let capacity = header.matches(',').count() + 1;
    let mut out: Vec<Candidate> = Vec::with_capacity(capacity);
    for entry in header.split(',') {
        if let Some(c) = parse_entry(entry.trim()) {
            if c.quality > 0.0 {
                out.push(c);
            }
        }
    }
    // Stable sort by descending q-value (preserves header order on ties)
    out.sort_by(|a, b| {
        b.quality
            .partial_cmp(&a.quality)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Parse a single `<tag>[;q=<weight>]` entry.
///
/// Returns `None` for empty entries or entries with malformed q-values
/// (per RFC 7231 §5.3.1).
fn parse_entry(entry: &str) -> Option<Candidate> {
    if entry.is_empty() {
        return None;
    }
    let mut parts = entry.split(';');
    let tag = parts.next()?.trim().to_string();
    if tag.is_empty() {
        return None;
    }
    let mut quality = 1.0_f32;
    for param in parts {
        let param = param.trim();
        // q-parameter is case-insensitive on the parameter name (RFC 7230
        // token is case-insensitive). Some proxies emit `Q=` instead of `q=`.
        let lower = param.to_ascii_lowercase();
        if let Some(q) = lower.strip_prefix("q=") {
            // Malformed q-value invalidates the entire entry (RFC 7231 §5.3.1)
            quality = parse_qvalue(q)?;
        }
    }
    Some(Candidate { tag, quality })
}

/// Parse a q-value string (`1.0`, `0.9`, `0`, `0.001`) into f32.
///
/// Per RFC 7231 §5.3.1, q-values are 0–1 with up to 3 decimal places.
/// Out-of-range or unparseable values return `None` (caller should drop
/// the entire entry).
fn parse_qvalue(s: &str) -> Option<f32> {
    let s = s.trim();
    let v: f32 = s.parse().ok()?;
    if !(0.0..=1.0).contains(&v) {
        return None;
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    /// Helper: negotiate from a `&str` header value.
    fn negotiate(s: &str) -> Locale {
        negotiate_locale_str(s)
    }

    /// Helper: negotiate from a `HeaderValue`.
    fn negotiate_hdr(s: &str) -> Locale {
        negotiate_locale(Some(&HeaderValue::try_from(s).unwrap()))
    }

    // ========== Locale::from_tag tests ==========

    #[test]
    fn test_from_tag_exact_en() {
        assert_eq!(Locale::from_tag("en"), Some(Locale::En));
        assert_eq!(Locale::from_tag("EN"), Some(Locale::En));
    }

    #[test]
    fn test_from_tag_exact_zh_cn() {
        assert_eq!(Locale::from_tag("zh-CN"), Some(Locale::ZhCn));
        assert_eq!(Locale::from_tag("zh-cn"), Some(Locale::ZhCn));
        assert_eq!(Locale::from_tag("ZH-CN"), Some(Locale::ZhCn));
    }

    #[test]
    fn test_from_tag_zh_hans_variants() {
        // zh-Hans and zh-Hans-CN should map to ZhCn
        assert_eq!(Locale::from_tag("zh-Hans"), Some(Locale::ZhCn));
        assert_eq!(Locale::from_tag("zh-Hans-CN"), Some(Locale::ZhCn));
    }

    #[test]
    fn test_from_tag_prefix_match() {
        // Prefix match: bare "zh" maps to ZhCn, bare "en" maps to En
        assert_eq!(Locale::from_tag("zh"), Some(Locale::ZhCn));
        assert_eq!(Locale::from_tag("en-US"), Some(Locale::En));
        assert_eq!(Locale::from_tag("en-GB"), Some(Locale::En));
        assert_eq!(Locale::from_tag("zh-TW"), Some(Locale::ZhCn));
    }

    #[test]
    fn test_from_tag_unsupported() {
        assert_eq!(Locale::from_tag("fr"), None);
        assert_eq!(Locale::from_tag("ja"), None);
        assert_eq!(Locale::from_tag("de-DE"), None);
        assert_eq!(Locale::from_tag(""), None);
    }

    // ========== Locale::as_str / Display / Default ==========

    #[test]
    fn test_locale_as_str() {
        assert_eq!(Locale::En.as_str(), "en");
        assert_eq!(Locale::ZhCn.as_str(), "zh-CN");
    }

    #[test]
    fn test_locale_display() {
        assert_eq!(format!("{}", Locale::En), "en");
        assert_eq!(format!("{}", Locale::ZhCn), "zh-CN");
    }

    #[test]
    fn test_locale_default_is_en() {
        assert_eq!(Locale::default(), Locale::En);
        assert_eq!(Locale::DEFAULT, Locale::En);
    }

    // ========== negotiate_locale_str tests (RFC 7231 §5.3.5) ==========

    #[test]
    fn test_negotiate_exact_zh_cn() {
        assert_eq!(negotiate("zh-CN"), Locale::ZhCn);
    }

    #[test]
    fn test_negotiate_exact_en() {
        assert_eq!(negotiate("en"), Locale::En);
    }

    #[test]
    fn test_negotiate_qvalue_sorting() {
        // zh-CN has lower q than en, so en wins
        assert_eq!(negotiate("zh-CN;q=0.9, en;q=0.8"), Locale::ZhCn);
        // Reverse: en first with higher q
        assert_eq!(negotiate("en;q=0.8, zh-CN;q=0.9"), Locale::ZhCn);
        // Equal q: header order wins (en first)
        assert_eq!(negotiate("en, zh-CN"), Locale::En);
        // Equal q: header order wins (zh-CN first)
        assert_eq!(negotiate("zh-CN, en"), Locale::ZhCn);
    }

    #[test]
    fn test_negotiate_qvalue_zero_drops() {
        // q=0 means "not acceptable"
        assert_eq!(negotiate("zh-CN;q=0, en;q=0.8"), Locale::En);
        // All q=0: falls back to default
        assert_eq!(negotiate("zh-CN;q=0, en;q=0"), Locale::En);
    }

    #[test]
    fn test_negotiate_prefix_match_zh() {
        assert_eq!(negotiate("zh"), Locale::ZhCn);
        assert_eq!(negotiate("zh-TW"), Locale::ZhCn);
        assert_eq!(negotiate("zh-HK;q=0.9, en;q=0.5"), Locale::ZhCn);
    }

    #[test]
    fn test_negotiate_prefix_match_en() {
        assert_eq!(negotiate("en-US"), Locale::En);
        assert_eq!(negotiate("en-GB"), Locale::En);
    }

    #[test]
    fn test_negotiate_unsupported_falls_back_to_default() {
        assert_eq!(negotiate("fr"), Locale::En);
        assert_eq!(negotiate("ja, ko"), Locale::En);
        assert_eq!(negotiate("de-DE, fr-FR"), Locale::En);
    }

    #[test]
    fn test_negotiate_missing_header() {
        // None header → default
        assert_eq!(negotiate_locale(None), Locale::En);
    }

    #[test]
    fn test_negotiate_empty_string() {
        assert_eq!(negotiate(""), Locale::En);
    }

    #[test]
    fn test_negotiate_malformed_header() {
        // Malformed entries should be skipped, not crash
        assert_eq!(negotiate(",,"), Locale::En);
        assert_eq!(negotiate(";q=0.5"), Locale::En);
        assert_eq!(negotiate("fr;badparam, en"), Locale::En);
    }

    #[test]
    fn test_negotiate_wildcard() {
        // Wildcard `*` matches nothing concrete, falls back to default
        assert_eq!(negotiate("*"), Locale::En);
        // Wildcard with q-value still matches nothing concrete
        assert_eq!(negotiate("zh-CN;q=0, *;q=0.5"), Locale::En);
    }

    #[test]
    fn test_negotiate_via_header_value() {
        assert_eq!(negotiate_hdr("zh-CN,zh;q=0.9,en;q=0.8"), Locale::ZhCn);
        assert_eq!(negotiate_hdr("en-US,en;q=0.9"), Locale::En);
        assert_eq!(negotiate_hdr(""), Locale::En);
    }

    #[test]
    fn test_negotiate_invalid_header_value_bytes() {
        // HeaderValue containing non-ASCII bytes → falls back to default
        let bad = HeaderValue::from_bytes(b"zh-CN\xff\xfe").unwrap();
        assert_eq!(negotiate_locale(Some(&bad)), Locale::En);
    }

    #[test]
    fn test_negotiate_qvalue_three_decimals() {
        // q=0.001 is the minimum positive q-value per RFC
        assert_eq!(negotiate("fr;q=0.001, en;q=0.000"), Locale::En);
        // fr with tiny q-value, en excluded → fr not supported → default
        assert_eq!(negotiate("fr;q=0.001"), Locale::En);
    }

    #[test]
    fn test_negotiate_qvalue_out_of_range_drops_entry() {
        // q > 1.0 is invalid → entire entry dropped (RFC 7231 §5.3.1)
        assert_eq!(negotiate("zh-CN;q=2.0, en;q=0.5"), Locale::En);
        // Negative q is also invalid → entry dropped
        assert_eq!(negotiate("zh-CN;q=-1.0, en;q=0.5"), Locale::En);
    }

    #[test]
    fn test_negotiate_malformed_qvalue_drops_entry() {
        // Malformed q-value invalidates the entire entry (RFC 7231 §5.3.1)
        // fr;q=abc is dropped, en;q=0.1 wins
        assert_eq!(negotiate("fr;q=abc, en;q=0.1"), Locale::En);
        // zh-CN;q=abc is dropped, falls to default
        assert_eq!(negotiate("zh-CN;q=abc"), Locale::En);
    }

    #[test]
    fn test_negotiate_q_param_case_insensitive() {
        // Q= (uppercase) should be treated same as q=
        assert_eq!(negotiate("zh-CN;Q=0.5, en;Q=0.9"), Locale::En);
        assert_eq!(negotiate("zh-CN;Q=0.9, en;Q=0.5"), Locale::ZhCn);
    }

    #[test]
    fn test_negotiate_header_too_long_returns_default() {
        // Header > 4 KiB → empty candidates → default
        let huge = "en, ".repeat(3000); // ~12 KiB
        assert_eq!(negotiate(&huge), Locale::En);
    }

    // ========== End-to-end middleware test ==========

    #[tokio::test]
    async fn test_locale_middleware_injects_extension() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use axum::routing::get;
        use axum::Router;
        use tower::ServiceExt;

        let app = Router::new()
            .route(
                "/locale",
                get(|req: Request<Body>| async move {
                    let locale = req
                        .extensions()
                        .get::<Locale>()
                        .copied()
                        .unwrap_or_default();
                    locale.as_str().to_string()
                }),
            )
            .layer(axum::middleware::from_fn(locale_middleware));

        // zh-CN request
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/locale")
                    .header("Accept-Language", "zh-CN")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"zh-CN");

        // Missing header → default
        let app2 = Router::new()
            .route(
                "/locale",
                get(|req: Request<Body>| async move {
                    let locale = req
                        .extensions()
                        .get::<Locale>()
                        .copied()
                        .unwrap_or_default();
                    locale.as_str().to_string()
                }),
            )
            .layer(axum::middleware::from_fn(locale_middleware));
        let resp = app2
            .oneshot(
                Request::builder()
                    .uri("/locale")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"en");
    }
}
