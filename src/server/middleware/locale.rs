// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Locale negotiation middleware：解析 `Accept-Language`（RFC 7231 §5.3.5），
//! 选出 `en` / `zh-CN` 并注入 `Extension<Locale>`；缺失/非法回退 `en`。
//! 安全：`Locale` 来自用户输入，禁止用于鉴权/认证等安全决策。

use axum::body::Body;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::fmt;

/// Supported locales for NebulaID.
///
/// 变体对应 `locales/{en,zh}/messages.ftl` 顶层键；由用户输入派生，禁止用于鉴权。
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

    /// 返回 `core::i18n` 消费的字符串标识（"en" / "zh-CN"）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::ZhCn => "zh-CN",
        }
    }

    /// 解析语言标签（大小写不敏感）：`zh*` → `ZhCn`，`en*` → `En`，未知返回 `None`。
    pub fn from_tag(tag: &str) -> Option<Self> {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            return None;
        }
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

/// Axum middleware：从 `Accept-Language` 协商 locale 并注入 `Extension<Locale>`。
pub async fn locale_middleware(mut req: Request<Body>, next: Next) -> Response {
    let locale = negotiate_locale(req.headers().get(axum::http::header::ACCEPT_LANGUAGE));
    req.extensions_mut().insert(locale);
    next.run(req).await
}

/// 从 `Accept-Language` 头协商 `Locale`；头缺失返回 `Locale::DEFAULT`。
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
        if let Some(locale) = Locale::from_tag(candidate.tag) {
            return locale;
        }
    }
    Locale::DEFAULT
}

/// A single entry in the `Accept-Language` header.
///
/// `tag` 借用头部字符串，避免逐候选 `String` 分配。
#[derive(Debug, PartialEq)]
struct Candidate<'a> {
    tag: &'a str,
    quality: f32,
}

/// `Accept-Language` 头部长度上限（4 KiB），防御超长头部 DoS。
const MAX_ACCEPT_LANGUAGE_LEN: usize = 4096;

/// 解析 `Accept-Language` 为按 q 值降序（稳定排序，同值保留头部顺序）的候选列表。
///
/// 丢弃 `q=0` 与 q 值非法条目（RFC 7231 §5.3.1）；空/超长/无有效条目返回空 `Vec`。
fn parse_accept_language<'a>(header: &'a str) -> Vec<Candidate<'a>> {
    if header.trim().is_empty() {
        return Vec::new();
    }
    if header.len() > MAX_ACCEPT_LANGUAGE_LEN {
        return Vec::new();
    }
    // 不预扫描逗号计数：典型头部仅 2–3 个候选，增长式扩容比
    // 二次 O(n) 扫描更便宜，4 KiB 病态输入下的重分配成本也有界。
    let mut out: Vec<Candidate<'a>> = Vec::new();
    for entry in header.split(',') {
        if let Some(c) = parse_entry(entry.trim()) {
            if c.quality > 0.0 {
                out.push(c);
            }
        }
    }
    // 按 q 值降序稳定排序（同值保留头部顺序）
    out.sort_by(|a, b| {
        b.quality
            .partial_cmp(&a.quality)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// 解析单个 `<tag>[;q=<weight>]` 条目；空条目或 q 值非法返回 `None`（RFC 7231 §5.3.1）。
fn parse_entry<'a>(entry: &'a str) -> Option<Candidate<'a>> {
    if entry.is_empty() {
        return None;
    }
    let mut parts = entry.split(';');
    let tag = parts.next()?.trim();
    if tag.is_empty() {
        return None;
    }
    let mut quality = 1.0_f32;
    for param in parts {
        let param = param.trim();
        // q 参数名大小写不敏感（RFC 7230），部分代理会发 `Q=`；比较前两字节
        // 避免分配，只检查前导 ASCII 字节，不会切到多字节字符中间。
        let bytes = param.as_bytes();
        if bytes.len() >= 2 && bytes[0].eq_ignore_ascii_case(&b'q') && bytes[1] == b'=' {
            // 余下部分从字节 2（ASCII 边界）起解析。
            // q 值非法则整条条目无效（RFC 7231 §5.3.1）
            quality = parse_qvalue(&param[2..])?;
        }
    }
    Some(Candidate { tag, quality })
}

/// 解析 q 值（0–1，最多 3 位小数）；越界或不可解析返回 `None`（RFC 7231 §5.3.1）。
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
        assert_eq!(Locale::from_tag("zh-Hans"), Some(Locale::ZhCn));
        assert_eq!(Locale::from_tag("zh-Hans-CN"), Some(Locale::ZhCn));
    }

    #[test]
    fn test_from_tag_prefix_match() {
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
        assert_eq!(negotiate("zh-CN;q=0.9, en;q=0.8"), Locale::ZhCn);
        assert_eq!(negotiate("en;q=0.8, zh-CN;q=0.9"), Locale::ZhCn);
        assert_eq!(negotiate("en, zh-CN"), Locale::En);
        assert_eq!(negotiate("zh-CN, en"), Locale::ZhCn);
    }

    #[test]
    fn test_negotiate_qvalue_zero_drops() {
        assert_eq!(negotiate("zh-CN;q=0, en;q=0.8"), Locale::En);
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
        assert_eq!(negotiate_locale(None), Locale::En);
    }

    #[test]
    fn test_negotiate_empty_string() {
        assert_eq!(negotiate(""), Locale::En);
    }

    #[test]
    fn test_negotiate_malformed_header() {
        assert_eq!(negotiate(",,"), Locale::En);
        assert_eq!(negotiate(";q=0.5"), Locale::En);
        assert_eq!(negotiate("fr;badparam, en"), Locale::En);
    }

    #[test]
    fn test_negotiate_wildcard() {
        assert_eq!(negotiate("*"), Locale::En);
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
        // 0.001 是 RFC 规定的最小正 q 值
        assert_eq!(negotiate("fr;q=0.001, en;q=0.000"), Locale::En);
        assert_eq!(negotiate("fr;q=0.001"), Locale::En);
    }

    #[test]
    fn test_negotiate_qvalue_out_of_range_drops_entry() {
        // q > 1.0 非法，整条条目丢弃（RFC 7231 §5.3.1）
        assert_eq!(negotiate("zh-CN;q=2.0, en;q=0.5"), Locale::En);
        assert_eq!(negotiate("zh-CN;q=-1.0, en;q=0.5"), Locale::En);
    }

    #[test]
    fn test_negotiate_malformed_qvalue_drops_entry() {
        // q 值非法导致整条条目失效（RFC 7231 §5.3.1）
        assert_eq!(negotiate("fr;q=abc, en;q=0.1"), Locale::En);
        assert_eq!(negotiate("zh-CN;q=abc"), Locale::En);
    }

    #[test]
    fn test_negotiate_q_param_case_insensitive() {
        assert_eq!(negotiate("zh-CN;Q=0.5, en;Q=0.9"), Locale::En);
        assert_eq!(negotiate("zh-CN;Q=0.9, en;Q=0.5"), Locale::ZhCn);
    }

    #[test]
    fn test_negotiate_header_too_long_returns_default() {
        let huge = "en, ".repeat(3000); // ~12 KiB
        assert_eq!(negotiate(&huge), Locale::En);
    }

    // ========== parse_accept_language / parse_entry unit tests ==========

    /// 验证 `parse_accept_language` 返回借用输入的 `Candidate<'a>`（不逐条分配）：
    /// 若 `Candidate` 改为 own `String`，下方 `_accept_borrowed` 将无法编译。
    #[test]
    fn test_parse_accept_language_returns_borrowed_candidates() {
        let header = String::from("en;q=0.9, zh-CN;q=0.8, *;q=0.1");
        let candidates = parse_accept_language(&header);
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].tag, "en");
        assert!((candidates[0].quality - 0.9).abs() < 1e-6);
        assert_eq!(candidates[1].tag, "zh-CN");
        assert!((candidates[1].quality - 0.8).abs() < 1e-6);
        assert_eq!(candidates[2].tag, "*");
        assert!((candidates[2].quality - 0.1).abs() < 1e-6);

        // 静态检查：helper 接受 `Vec<Candidate<'a>>`，若 Candidate own String
        // 则生命周期不匹配、无法编译，防止回归到逐条分配实现。
        fn _accept_borrowed<'a>(_c: Vec<Candidate<'a>>) {}
        _accept_borrowed(candidates);
    }

    /// `q=0` 条目必须丢弃（RFC 7231 §5.3.1，显式不可接受）。
    #[test]
    fn test_parse_accept_language_drops_q_zero() {
        let candidates = parse_accept_language("en;q=0, zh-CN;q=0.9");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].tag, "zh-CN");
    }

    /// q 参数名大小写不敏感（RFC 7230），`Q=` 与 `q=` 必须等价。
    #[test]
    fn test_parse_entry_q_param_case_insensitive() {
        let lower = parse_entry("zh-CN;q=0.9").unwrap();
        assert_eq!(lower.tag, "zh-CN");
        assert!((lower.quality - 0.9).abs() < 1e-6);

        let upper = parse_entry("zh-CN;Q=0.5").unwrap();
        assert_eq!(upper.tag, "zh-CN");
        assert!((upper.quality - 0.5).abs() < 1e-6);

        let mixed = parse_entry("zh-CN;q=0.7").unwrap();
        assert!((mixed.quality - 0.7).abs() < 1e-6);
    }

    /// `q=` 值非法时整条条目丢弃（RFC 7231 §5.3.1）。
    #[test]
    fn test_parse_entry_malformed_q_drops_entry() {
        assert!(parse_entry("zh-CN;q=abc").is_none());
        assert!(parse_entry("zh-CN;q=2.0").is_none());
        assert!(parse_entry("zh-CN;q=-1.0").is_none());
        // Missing `=` after `q` → not a q-param, entry kept with q=1.0
        let kept = parse_entry("zh-CN;qxyz").unwrap();
        assert!((kept.quality - 1.0).abs() < 1e-6);
    }

    /// 空 / 纯空白条目丢弃。
    #[test]
    fn test_parse_entry_empty_drops() {
        assert!(parse_entry("").is_none());
        assert!(parse_entry("   ").is_none());
        assert!(parse_entry(";q=0.5").is_none());
    }

    // ========== End-to-end middleware test ==========

    #[tokio::test]
    async fn test_locale_middleware_injects_extension() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use axum::routing::get;
        use axum::Router;
        use sdforge::tower::ServiceExt;

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
