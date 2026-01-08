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

use axum::Router;
use utoipa::OpenApi;

use crate::models::{
    ApiErrorResponse, ApiInfoResponse, ApiKeyListResponse, ApiKeyResponse,
    ApiKeyWithSecretResponse, BatchGenerateRequest, BatchGenerateResponse, BizTagListResponse,
    BizTagResponse, CreateApiKeyRequest, CreateBizTagRequest, CreateGroupRequest,
    CreateWorkspaceRequest, ErrorResponse, GenerateRequest, GenerateResponse, GroupListResponse,
    GroupResponse, HealthResponse, MetricsResponse, PaginationParams, ParseRequest, ParseResponse,
    ReadyResponse, RevokeApiKeyResponse, SecureConfigResponse, SetAlgorithmRequest,
    SetAlgorithmResponse, UpdateBizTagRequest, UpdateConfigResponse, UpdateLoggingRequest,
    UpdateRateLimitRequest, WorkspaceListResponse, WorkspaceResponse,
};

/// OpenAPI 文档定义
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Nebula ID API",
        version = "1.0.0",
        description = concat!(
            "# Nebula ID Service API\n\n",
            "Enterprise-grade distributed ID generation system supporting multiple algorithms:\n",
            "- **Segment**: Database-based segment allocation, high throughput, ordered\n",
            "- **Snowflake**: Twitter Snowflake variant, distributed unique, time-ordered\n",
            "- **UUID v7**: Time-sorted UUID standard\n",
            "- **UUID v4**: Random UUID fallback\n\n",
            "## Authentication\n\n",
            "All authenticated endpoints require an API Key in the `Authorization` header:\n",
            "```\n",
            "Authorization: Basic base64(key_id:key_secret)\n",
            "Authorization: ApiKey key_id:key_secret\n",
            "```\n\n",
            "## Rate Limiting\n\n",
            "Default rate limit: 1000 requests/second per API key\n",
            "Burst size: 100 requests\n\n",
            "## Error Codes\n\n",
            "| Code | Description |\n",
            "|------|-------------|\n",
            "| 1001 | Unauthorized - Missing or invalid API key |\n",
            "| 1002 | Forbidden - Insufficient permissions |\n",
            "| 1003 | Invalid API Key format |\n",
            "| 1004 | API Key expired |\n",
            "| 1005 | API Key disabled |\n",
            "| 2001 | Workspace not found |\n",
            "| 2002 | Group not found |\n",
            "| 2003 | BizTag not found |\n",
            "| 2004 | Resource already exists |\n",
            "| 3001 | Invalid input |\n",
            "| 3002 | Validation error |\n",
            "| 3003 | Missing required field |\n",
            "| 3004 | Invalid UUID |\n",
            "| 4001 | Rate limit exceeded |\n",
            "| 5001 | Internal server error |\n",
            "| 5002 | Database error |\n",
            "| 5003 | Cache error |\n",
            "| 5004 | Service unavailable |"
        ),
        contact(
            name = "Kirky.X",
            email = "support@nebulaid.io",
        ),
        license(
            name = "Apache-2.0",
            url = "https://www.apache.org/licenses/LICENSE-2.0",
        )
    ),
    paths(
        crate::openapi::swagger_ui_handler,
        crate::openapi::openapi_json_handler,
    ),
    components(
        schemas(
            ApiErrorResponse,
            ApiInfoResponse,
            ApiKeyListResponse,
            ApiKeyResponse,
            ApiKeyWithSecretResponse,
            BatchGenerateRequest,
            BatchGenerateResponse,
            BizTagListResponse,
            BizTagResponse,
            CreateApiKeyRequest,
            CreateBizTagRequest,
            CreateGroupRequest,
            CreateWorkspaceRequest,
            ErrorResponse,
            GenerateRequest,
            GenerateResponse,
            GroupListResponse,
            GroupResponse,
            HealthResponse,
            MetricsResponse,
            PaginationParams,
            ParseRequest,
            ParseResponse,
            ReadyResponse,
            RevokeApiKeyResponse,
            SecureConfigResponse,
            SetAlgorithmRequest,
            SetAlgorithmResponse,
            UpdateBizTagRequest,
            UpdateConfigResponse,
            UpdateLoggingRequest,
            UpdateRateLimitRequest,
            WorkspaceListResponse,
            WorkspaceResponse,
        )
    ),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "id", description = "ID generation and parsing"),
        (name = "config", description = "Configuration management"),
        (name = "admin", description = "Admin-only operations"),
        (name = "workspaces", description = "Workspace management"),
        (name = "groups", description = "Group management"),
        (name = "biz_tags", description = "Business tag management"),
        (name = "api_keys", description = "API key management"),
    )
)]
pub struct ApiDoc;

/// 创建 Swagger UI 路由
pub fn create_swagger_router() -> Router {
    // 临时禁用 Swagger UI 以解决编译问题
    // TODO: 修复 utoipa-swagger-ui 集成
    Router::new()
}

/// Swagger UI 处理器（用于 OpenAPI 文档生成）
#[utoipa::path(
    get,
    path = "/api/docs/swagger-ui",
    context_path = "/api/docs",
    responses(
        (status = 200, description = "Swagger UI page", body = String),
    ),
    tag = "health"
)]
pub async fn swagger_ui_handler() -> String {
    String::from("Swagger UI")
}

/// OpenAPI JSON 处理器（用于 OpenAPI 规范生成）
#[utoipa::path(
    get,
    path = "/api/docs/openapi.json",
    context_path = "/api/docs",
    responses(
        (status = 200, description = "OpenAPI specification", content_type = "application/json"),
    ),
    tag = "health"
)]
pub async fn openapi_json_handler() -> String {
    String::from("OpenAPI JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_doc_serialization() {
        let openapi = ApiDoc::openapi();
        let json = serde_json::to_string(&openapi);
        assert!(json.is_ok());
    }

    #[test]
    fn test_swagger_router_creation() {
        let _router = create_swagger_router();
    }
}
