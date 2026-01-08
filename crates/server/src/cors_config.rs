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

/*! CORS 配置模块

该模块提供 CORS (跨源资源共享) 配置功能，包括：
- 生产级 CORS 配置
- 环境感知的 CORS 策略
- 安全头暴露
- 预检缓存配置
*/

use axum::http::{HeaderName, HeaderValue, Method};
use std::time::Duration;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

/// CORS 预检请求缓存时间（1小时）
const CORS_MAX_AGE: Duration = Duration::from_secs(3600);

/// 暴露的响应头
pub const EXPOSED_HEADERS: [&str; 2] = ["x-request-id", "x-rate-limit-remaining"];

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
/// # 参数
/// * `allowed_origins` - 允许的源列表（如果为空，则拒绝所有请求）
///
/// # 返回
/// 返回配置好的 CorsLayer
///
/// # 示例
/// ```rust
/// # use nebula_server::cors_config::create_cors_layer;
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

/// 创建开发环境的 CORS 配置
///
/// 允许 localhost 用于开发测试
///
/// # 返回
/// 返回配置宽松的 CorsLayer，仅用于开发环境
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

/// 创建环境感知的 CORS 配置
///
/// 从环境变量读取配置：
/// - `ALLOWED_ORIGINS`: 逗号分隔的允许源列表
/// - `NEBULA_ENV`: 环境类型（production/development）
///
/// # 返回
/// 返回根据环境配置的 CorsLayer
pub fn create_env_aware_cors_layer() -> CorsLayer {
    let is_production = std::env::var("NEBULA_ENV")
        .unwrap_or_else(|_| "development".to_string())
        .to_lowercase()
        == "production";

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
            tracing::error!("ALLOWED_ORIGINS must be configured in production environment");
            panic!(
                "Security: ALLOWED_ORIGINS environment variable is required in production. \
                 Example: ALLOWED_ORIGINS=https://example.com,https://app.example.com"
            );
        }
        create_cors_layer(allowed_origins)
    } else {
        // 开发环境使用更严格的默认配置
        if allowed_origins.is_empty() {
            tracing::warn!(
                "ALLOWED_ORIGINS not configured in development, using default localhost origins"
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

        // Verify that the layer can be created without panicking
        let _ = cors;
    }

    #[test]
    fn test_create_cors_layer_empty_origins() {
        let cors = create_cors_layer(vec![]);

        // Verify that empty origins list is handled
        let _ = cors;
    }

    #[test]
    fn test_create_cors_layer_invalid_origins() {
        let origins = vec![
            "https://example.com".to_string(),
            "invalid-origin".to_string(), // Invalid URL
        ];
        let cors = create_cors_layer(origins);

        // Invalid origins should be filtered out
        let _ = cors;
    }

    #[test]
    fn test_create_dev_cors_layer() {
        let cors = create_dev_cors_layer();

        // Verify that dev layer can be created
        let _ = cors;
    }

    #[test]
    fn test_exposed_headers_const() {
        assert_eq!(EXPOSED_HEADERS.len(), 2);
        assert_eq!(EXPOSED_HEADERS[0], "x-request-id");
        assert_eq!(EXPOSED_HEADERS[1], "x-rate-limit-remaining");
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
