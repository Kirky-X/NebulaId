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

/*! API 版本控制模块

该模块提供 API 版本化功能，包括：
- 版本常量定义
- 版本解析和验证
- 版本响应头管理
*/

use axum::{
    body::Body,
    http::{HeaderName, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::str::FromStr;

/// API 版本常量
pub const API_V1: &str = "v1";
pub const API_V2: &str = "v2";
pub const CURRENT_API_VERSION: &str = API_V1;
pub const DEFAULT_API_VERSION_HEADER: &str = "X-API-Version";

/// API 版本请求头名称
pub static API_VERSION_HEADER: HeaderName = HeaderName::from_static("x-api-version");

/// API 版本号
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiVersion {
    /// 版本 1（当前稳定版本）
    V1,
    /// 版本 2（未来版本，预留）
    V2,
}

impl ApiVersion {
    /// 获取版本字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiVersion::V1 => API_V1,
            ApiVersion::V2 => API_V2,
        }
    }

    /// 检查版本是否受支持
    pub fn is_supported(&self) -> bool {
        matches!(self, ApiVersion::V1)
    }

    /// 获取版本号（数字形式）
    pub fn as_number(&self) -> u32 {
        match self {
            ApiVersion::V1 => 1,
            ApiVersion::V2 => 2,
        }
    }
}

impl Default for ApiVersion {
    fn default() -> Self {
        ApiVersion::V1
    }
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ApiVersion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "v1" | "1" => Ok(ApiVersion::V1),
            "v2" | "2" => Ok(ApiVersion::V2),
            other => Err(format!("Unsupported API version: {}", other)),
        }
    }
}

impl TryFrom<&str> for ApiVersion {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl TryFrom<String> for ApiVersion {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_str(&value)
    }
}

/// 从请求中解析 API 版本
pub fn parse_api_version_from_request(req: &Request<Body>) -> ApiVersion {
    req.headers()
        .get(&API_VERSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| ApiVersion::from_str(s).ok())
        .unwrap_or_default()
}

/// API 版本中间件
///
/// 这个中间件执行以下操作：
/// 1. 尝试从请求头 `X-API-Version` 读取版本
/// 2. 如果没有提供，使用默认版本 (v1)
/// 3. 验证版本是否受支持
/// 4. 在响应头中添加版本信息
pub async fn api_version_middleware(
    req: Request<Body>,
    next: Next,
) -> Result<Response, ApiVersionErrorResponse> {
    // 从请求头获取版本，如果不存在则使用默认版本
    let version = req
        .headers()
        .get(&API_VERSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| ApiVersion::from_str(s).ok())
        .unwrap_or_default();

    // 检查版本是否受支持
    if !version.is_supported() {
        return Err(ApiVersionErrorResponse::UnsupportedVersion {
            requested: version.as_str().to_string(),
            supported_versions: vec![API_V1.to_string()],
        });
    }

    // 继续处理请求
    let mut response = next.run(req).await;

    // 在响应头中添加 API 版本
    response
        .headers_mut()
        .insert(&API_VERSION_HEADER, version.as_str().parse().unwrap());

    Ok(response)
}

/// API 版本错误响应
#[derive(Debug)]
pub enum ApiVersionErrorResponse {
    /// 不支持的版本
    UnsupportedVersion {
        requested: String,
        supported_versions: Vec<String>,
    },
}

impl IntoResponse for ApiVersionErrorResponse {
    fn into_response(self) -> Response {
        use crate::models::{ApiErrorCode, ApiErrorResponse};

        match self {
            ApiVersionErrorResponse::UnsupportedVersion {
                requested,
                supported_versions,
            } => {
                let error_response = ApiErrorResponse::new(
                    ApiErrorCode::InvalidInput,
                    "Unsupported API version".to_string(),
                )
                .with_details(format!(
                    "Requested version: '{}'. Supported versions: {}",
                    requested,
                    supported_versions.join(", ")
                ));

                (StatusCode::BAD_REQUEST, axum::Json(error_response)).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_version_display() {
        assert_eq!(ApiVersion::V1.to_string(), "v1");
        assert_eq!(ApiVersion::V2.to_string(), "v2");
    }

    #[test]
    fn test_api_version_from_str() {
        assert_eq!(ApiVersion::from_str("v1").unwrap(), ApiVersion::V1);
        assert_eq!(ApiVersion::from_str("V1").unwrap(), ApiVersion::V1);
        assert_eq!(ApiVersion::from_str("1").unwrap(), ApiVersion::V1);
        assert_eq!(ApiVersion::from_str("v2").unwrap(), ApiVersion::V2);
        assert_eq!(ApiVersion::from_str("2").unwrap(), ApiVersion::V2);
        assert!(ApiVersion::from_str("v3").is_err());
    }

    #[test]
    fn test_api_version_as_str() {
        assert_eq!(ApiVersion::V1.as_str(), "v1");
        assert_eq!(ApiVersion::V2.as_str(), "v2");
    }

    #[test]
    fn test_api_version_as_number() {
        assert_eq!(ApiVersion::V1.as_number(), 1);
        assert_eq!(ApiVersion::V2.as_number(), 2);
    }

    #[test]
    fn test_api_version_default() {
        assert_eq!(ApiVersion::default(), ApiVersion::V1);
    }

    #[test]
    fn test_api_version_is_supported() {
        assert!(ApiVersion::V1.is_supported());
        assert!(!ApiVersion::V2.is_supported());
    }

    #[test]
    fn test_api_version_error_response() {
        let error = ApiVersionErrorResponse::UnsupportedVersion {
            requested: "v3".to_string(),
            supported_versions: vec!["v1".to_string()],
        };

        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_try_from_string() {
        assert_eq!(
            TryInto::<ApiVersion>::try_into("v1".to_string()).unwrap(),
            ApiVersion::V1
        );
        assert_eq!(
            TryInto::<ApiVersion>::try_into("V1".to_string()).unwrap(),
            ApiVersion::V1
        );
        assert_eq!(
            TryInto::<ApiVersion>::try_into("1".to_string()).unwrap(),
            ApiVersion::V1
        );
        assert!(TryInto::<ApiVersion>::try_into("v3".to_string()).is_err());
    }
}
