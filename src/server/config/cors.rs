// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

/*! CORS 配置模块：生产级配置、环境感知策略、安全头暴露与预检缓存。*/

use crate::server::rate_limit::middleware::HEADER_RATE_LIMIT_REMAINING;
use axum::http::{HeaderName, HeaderValue, Method};
use sdforge::tower_http::cors::{AllowOrigin, Any, CorsLayer};
use std::time::Duration;

/// CORS 预检请求缓存时间（1小时）
const CORS_MAX_AGE: Duration = Duration::from_secs(3600);

/// 暴露的响应头。
///
/// 限流条目必须引用中间件实际写出的常量：header 名连字符敏感，拼写不一致会被浏览器静默丢弃。
pub const EXPOSED_HEADERS: [&str; 2] = ["x-request-id", HEADER_RATE_LIMIT_REMAINING];

/// 支持的 HTTP 方法
const ALLOWED_METHODS: [Method; 5] = [
    Method::GET,
    Method::POST,
    Method::PUT,
    Method::DELETE,
    Method::PATCH,
];

/// 创建生产级 CORS 配置
///
/// `allowed_origins` 为空时拒绝所有请求。
///
/// # 示例
/// ```ignore
/// use nebulaid::server::config::cors::create_cors_layer;
/// let origins = vec![
///     "https://example.com".to_string(),
///     "https://app.example.com".to_string(),
/// ];
/// let cors = create_cors_layer(origins);
/// ```
pub fn create_cors_layer(allowed_origins: Vec<String>) -> CorsLayer {
    let origins: Vec<HeaderValue> = allowed_origins
        .into_iter()
        .filter_map(|s| s.parse().ok())
        .collect();

    // 如果没有配置允许的源，使用空列表（拒绝所有请求）
    let allow_origin = if origins.is_empty() {
        AllowOrigin::list(vec![])
    } else {
        AllowOrigin::list(origins)
    };

    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods(ALLOWED_METHODS)
        .allow_headers(Any)
        .allow_credentials(false) // 不允许携带凭据，提高安全性
        .max_age(CORS_MAX_AGE)
        .expose_headers(
            EXPOSED_HEADERS
                .iter()
                .map(|s| HeaderName::from_static(s))
                .collect::<Vec<_>>(),
        )
}

/// 创建开发环境的 CORS 配置：仅允许 localhost，仅供开发测试。
pub fn create_dev_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::list([
            "http://localhost:3000".parse().unwrap(),
            "http://127.0.0.1:3000".parse().unwrap(),
        ]))
        .allow_methods(ALLOWED_METHODS)
        .allow_headers(Any)
        .allow_credentials(false)
        .max_age(CORS_MAX_AGE)
        .expose_headers(
            EXPOSED_HEADERS
                .iter()
                .map(|s| HeaderName::from_static(s))
                .collect::<Vec<_>>(),
        )
}

/// 创建环境感知的 CORS 配置：读 `ALLOWED_ORIGINS`（逗号分隔）；
/// `NEBULA_ENV` 缺失或无法识别时按生产处理（fail-closed）。
pub fn create_env_aware_cors_layer() -> CorsLayer {
    // 环境判定唯一来源：core::config::is_production（NEBULA_ENV 缺失/未知
    // 值 fail-closed 按生产处理），不得自行回退 development 绕开生产校验。
    let is_production = crate::core::config::is_production();

    let allowed_origins: Vec<String> = std::env::var("ALLOWED_ORIGINS")
        .ok()
        .map(|origins| {
            origins
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    if is_production {
        // 生产环境必须显式配置允许的源
        if allowed_origins.is_empty() {
            tracing::error!(
                "{}",
                t!("log.server.config.cors.allowed_origins_required_in_production")
            );
            tracing::error!(
                "{}",
                t!("log.server.config.cors.allowed_origins_security_required")
            );
            tracing::error!("{}", t!("log.server.config.cors.allowed_origins_example"));
            return CorsLayer::new();
        }
        create_cors_layer(allowed_origins)
    } else {
        // 开发环境缺省时回落 localhost 白名单
        if allowed_origins.is_empty() {
            tracing::warn!(
                "{}",
                t!("log.server.config.cors.allowed_origins_dev_default")
            );
            create_dev_cors_layer()
        } else {
            create_cors_layer(allowed_origins)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_cors_layer_with_origins() {
        let origins = vec![
            "https://example.com".to_string(),
            "https://app.example.com".to_string(),
        ];
        let cors = create_cors_layer(origins);

        let _ = cors;
    }

    #[test]
    fn test_create_cors_layer_empty_origins() {
        let cors = create_cors_layer(vec![]);

        let _ = cors;
    }

    #[test]
    fn test_create_cors_layer_invalid_origins() {
        let origins = vec![
            "https://example.com".to_string(),
            "invalid-origin".to_string(), // Invalid URL
        ];
        let cors = create_cors_layer(origins);

        let _ = cors;
    }

    #[test]
    fn test_create_dev_cors_layer() {
        let cors = create_dev_cors_layer();

        let _ = cors;
    }

    #[test]
    fn test_exposed_headers_const() {
        assert_eq!(EXPOSED_HEADERS.len(), 2);
        assert_eq!(EXPOSED_HEADERS[0], "x-request-id");
        // 单一来源：暴露名必须等于限流中间件实际写出的 header 名
        assert_eq!(
            EXPOSED_HEADERS[1],
            crate::server::rate_limit::middleware::HEADER_RATE_LIMIT_REMAINING
        );
    }

    #[test]
    fn test_cors_max_age() {
        assert_eq!(CORS_MAX_AGE, Duration::from_secs(3600));
    }

    #[test]
    fn test_allowed_methods() {
        assert_eq!(ALLOWED_METHODS.len(), 5);
        assert!(ALLOWED_METHODS.contains(&Method::GET));
        assert!(ALLOWED_METHODS.contains(&Method::POST));
        assert!(ALLOWED_METHODS.contains(&Method::PUT));
        assert!(ALLOWED_METHODS.contains(&Method::DELETE));
        assert!(ALLOWED_METHODS.contains(&Method::PATCH));
    }
}
