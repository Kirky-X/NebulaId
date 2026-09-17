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
use sdforge::utoipa::OpenApi;

use crate::server::models::{
    ApiInfoResponse, ApiKeyListResponse, ApiKeyResponse, ApiKeyWithSecretResponse,
    BatchGenerateRequest, BatchGenerateResponse, BizTagListResponse, BizTagResponse,
    CreateApiKeyRequest, CreateBizTagRequest, CreateGroupRequest, CreateWorkspaceRequest,
    ErrorResponse, GenerateRequest, GenerateResponse, GroupListResponse, GroupResponse,
    HealthResponse, MetricsResponse, PaginationParams, ParseRequest, ParseResponse, ReadyResponse,
    RevokeApiKeyResponse, SecureConfigResponse, SetAlgorithmRequest, SetAlgorithmResponse,
    UpdateBizTagRequest, UpdateConfigResponse, UpdateLoggingRequest, UpdateRateLimitRequest,
    WorkspaceListResponse, WorkspaceResponse,
};

/// OpenAPI 文档定义
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Nebula ID API",
        version = env!("CARGO_PKG_VERSION"),
        description = concat!(
            "# Nebula ID Service API\n\n",
            "Enterprise-grade distributed ID generation system supporting multiple algorithms:\n",
            "- **Segment**: Database-based segment allocation, high throughput, ordered\n",
            "- **Snowflake**: Twitter Snowflake variant, distributed unique, time-ordered\n",
            "- **UUID v8**: Time-sorted UUID with custom layout (RFC 9562 §5.8)\n\n",
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
            "All error responses share one envelope (`ErrorResponse`):\n",
            "`{ code, business_code, message, details, request_id, timestamp }`.\n",
            "`business_code` is the stable machine-readable code below (string,\n",
            "e.g. `\"4001\"`); branch on it instead of parsing `message`.\n\n",
            "| business_code | Description |\n",
            "|---------------|-------------|\n",
            "| 1001 | Unauthorized - Missing or invalid API key |\n",
            "| 1002 | Forbidden - Insufficient permissions / cross-workspace access |\n",
            "| 1003 | Invalid API Key format |\n",
            "| 1004 | API Key expired |\n",
            "| 1005 | API Key disabled |\n",
            "| 2001 | Workspace not found (default for generic 404 resources) |\n",
            "| 2002 | Group not found (reserved, not currently returned) |\n",
            "| 2003 | BizTag not found |\n",
            "| 2004 | Resource already exists (reserved, not currently returned) |\n",
            "| 3001 | Invalid input |\n",
            "| 3002 | Validation error |\n",
            "| 3003 | Missing required field |\n",
            "| 3004 | Invalid UUID |\n",
            "| 4001 | Rate limit exceeded |\n",
            "| 5001 | Internal server error |\n",
            "| 5002 | Database error |\n",
            "| 5003 | Cache error |\n",
            "| 5004 | Service unavailable (timeout) |"
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
        crate::server::openapi::openapi_json_handler,
        // ===== system（公开探针 + api-info，注解载体见 handlers/system_handlers.rs）=====
        crate::server::handlers::system_handlers::health_docs,
        crate::server::handlers::system_handlers::ready_docs,
        crate::server::handlers::system_handlers::metrics_docs,
        crate::server::handlers::system_handlers::api_info_docs,
        // ===== ids（注解载体见 handlers/id_handlers.rs）=====
        crate::server::handlers::id_handlers::generate_docs,
        crate::server::handlers::id_handlers::batch_generate_docs,
        crate::server::handlers::id_handlers::parse_docs,
        // ===== config（注解载体见 handlers/system_handlers.rs）=====
        crate::server::handlers::system_handlers::get_config_docs,
        crate::server::handlers::system_handlers::update_rate_limit_docs,
        crate::server::handlers::system_handlers::update_logging_docs,
        crate::server::handlers::system_handlers::reload_config_docs,
        crate::server::handlers::system_handlers::set_algorithm_docs,
        // ===== workspaces / groups（注解载体见 handlers/workspace_handlers.rs）=====
        crate::server::handlers::workspace_handlers::create_workspace_docs,
        crate::server::handlers::workspace_handlers::list_workspaces_docs,
        crate::server::handlers::workspace_handlers::get_workspace_docs,
        crate::server::handlers::workspace_handlers::regenerate_user_key_docs,
        crate::server::handlers::workspace_handlers::create_group_docs,
        crate::server::handlers::workspace_handlers::list_groups_docs,
        // ===== biz-tags（注解载体见 handlers/biz_tag_handlers.rs）=====
        crate::server::handlers::biz_tag_handlers::create_biz_tag_docs,
        crate::server::handlers::biz_tag_handlers::list_biz_tags_docs,
        crate::server::handlers::biz_tag_handlers::get_biz_tag_docs,
        crate::server::handlers::biz_tag_handlers::update_biz_tag_docs,
        crate::server::handlers::biz_tag_handlers::delete_biz_tag_docs,
        // ===== api-keys（注解载体见 handlers/api_key_handlers.rs）=====
        crate::server::handlers::api_key_handlers::create_api_key_docs,
        crate::server::handlers::api_key_handlers::list_api_keys_docs,
        crate::server::handlers::api_key_handlers::revoke_api_key_docs,
    ),
    components(
        schemas(
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
        // T033 —— tag 体系与 handlers 注解载体的资源域对齐：
        // ids / workspaces / groups / biz-tags / api-keys / config / system / docs。
        // （admin 语义并入 api-keys / config 的端点描述与 403 响应说明。）
        (name = "ids", description = "ID generation and parsing"),
        (name = "workspaces", description = "Workspace management"),
        (name = "groups", description = "Group management"),
        (name = "biz-tags", description = "Business tag management"),
        (name = "api-keys", description = "API key management (admin only)"),
        (name = "config", description = "Configuration management (GET authenticated, mutations admin only)"),
        (name = "system", description = "Health, readiness, metrics and API info"),
        (name = "docs", description = "API documentation"),
    )
)]
pub struct ApiDoc;

/// 创建 Swagger UI 路由（预留）
pub fn create_swagger_router() -> Router {
    Router::new()
}

/// OpenAPI JSON 处理器
#[sdforge::utoipa::path(
    get,
    path = "/api-docs/openapi.json",
    responses(
        (status = 200, description = "OpenAPI specification", content_type = "application/json"),
    ),
    tag = "docs"
)]
pub async fn openapi_json_handler() -> impl axum::response::IntoResponse {
    axum::Json(ApiDoc::openapi())
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

    // ========== T033 —— paths 守卫 ==========

    /// `/api/v1` 业务路由清单（T033）—— openapi paths 与 `router.rs`
    /// 注册处的一一对应表（路径用 OpenAPI `{}` 语法；router.rs 注册处
    /// 为同名 `{}` 语法，语义一一对应；`/api/v1` 根以 `/api/v1/` 呈现）。
    ///
    /// 方案说明（选型见任务报告）： axum 0.8 `Router::routes()` 不展开
    /// `nest()` 的子路由，且完整装配 router 需要克隆整套 auth/audit/限流
    /// mock，故守卫不遍历运行时 Router，而是对齐既有 T005 parity 清单
    /// （`router.rs::API_V1_ROUTE_PREFIXES` / api-info endpoints）维护本表：
    /// - 正向：本表每条路径+方法都必须出现在 openapi 文档（漏注解即失败）；
    /// - 反向：openapi 文档中 `/api/` 前缀路径不得超出本表（注解漂移即失败）；
    /// - 上界：paths 总数 ≥ 表长（任务要求的「paths 数 ≥ 路由数」）。
    /// 新增 /api/v1 路由时 T005 守卫会先失败并把开发者引到 router.rs，
    /// 同步本表与注解即可。
    const EXPECTED_V1_PATHS: &[(&str, &[&str])] = &[
        ("/api/v1/", &["get"]),
        ("/api/v1/generate", &["post"]),
        ("/api/v1/generate/batch", &["post"]),
        ("/api/v1/parse", &["post"]),
        ("/api/v1/config", &["get"]),
        ("/api/v1/config/rate-limit", &["post"]),
        ("/api/v1/config/logging", &["post"]),
        ("/api/v1/config/reload", &["post"]),
        ("/api/v1/config/algorithm", &["post"]),
        ("/api/v1/workspaces", &["get", "post"]),
        ("/api/v1/workspaces/{name}", &["get"]),
        ("/api/v1/workspaces/{name}/regenerate-user-key", &["post"]),
        ("/api/v1/groups", &["get", "post"]),
        ("/api/v1/biz-tags", &["get", "post"]),
        ("/api/v1/biz-tags/{id}", &["get", "put", "delete"]),
        ("/api/v1/api-keys", &["get", "post"]),
        ("/api/v1/api-keys/{id}", &["delete"]),
    ];

    fn openapi_json() -> serde_json::Value {
        serde_json::to_value(ApiDoc::openapi()).expect("ApiDoc must serialize to JSON")
    }

    #[test]
    fn test_openapi_paths_cover_all_v1_routes() {
        let doc = openapi_json();
        let paths = doc["paths"].as_object().expect("paths must be an object");

        // 正向：清单内每条路径与方法都必须注册。
        for (path, methods) in EXPECTED_V1_PATHS {
            let item = paths
                .get(*path)
                .unwrap_or_else(|| panic!("openapi paths 缺少 v1 路由: {path}"));
            for method in *methods {
                assert!(
                    item.get(*method).is_some(),
                    "openapi 路由 {path} 缺少方法 {method}"
                );
            }
        }

        // 上界：paths 总数（含 /health、/ready、/metrics、/api-docs/openapi.json）
        // ≥ v1 清单长度。
        assert!(
            paths.len() >= EXPECTED_V1_PATHS.len(),
            "openapi paths 数量({}) 不得少于 v1 路由数({})",
            paths.len(),
            EXPECTED_V1_PATHS.len()
        );

        // 反向：/api/ 前缀路径不得超出清单（注解漂移检测）。
        let extra: Vec<&String> = paths
            .keys()
            .filter(|k| k.starts_with("/api/") && !EXPECTED_V1_PATHS.iter().any(|(p, _)| p == k))
            .collect();
        assert!(
            extra.is_empty(),
            "openapi 存在清单之外的 /api/ 路径: {extra:?}（请同步 EXPECTED_V1_PATHS 与 router.rs）"
        );
    }

    #[test]
    fn test_openapi_paths_contain_core_resource_keys() {
        let doc = openapi_json();
        let paths = doc["paths"].as_object().unwrap();
        for key in [
            "/api/v1/generate",
            "/api/v1/generate/batch",
            "/api/v1/parse",
            "/api/v1/workspaces",
            "/api/v1/groups",
            "/api/v1/biz-tags",
            "/api/v1/api-keys",
        ] {
            assert!(
                paths.contains_key(key),
                "openapi paths 必须包含 {key}（generate/batch/parse/workspaces/groups/biz-tags/api-keys）"
            );
        }
    }

    #[test]
    fn test_openapi_error_responses_reference_unified_envelope() {
        // T032 统一错误信封：受保护端点的 4xx/5xx 都应引用 $ref ErrorResponse。
        let doc = openapi_json();
        let generate = &doc["paths"]["/api/v1/generate"]["post"];
        let unauthorized = &generate["responses"]["401"];
        assert!(
            unauthorized["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .unwrap_or_default()
                .ends_with("ErrorResponse"),
            "401 响应应引用 ErrorResponse schema，实际: {unauthorized}"
        );
    }

    #[test]
    fn test_core_request_schemas_carry_examples() {
        // T033 —— 核心四 schema 必须带 example（与 docs/API_REFERENCE.md 同口径）。
        let doc = openapi_json();
        let schemas = &doc["components"]["schemas"];
        for name in [
            "GenerateRequest",
            "GenerateResponse",
            "ErrorResponse",
            "ParseResponse",
        ] {
            let schema = &schemas[name];
            assert!(
                schema.get("example").is_some(),
                "{name} 必须声明 struct 级 example"
            );
        }
        let ws = schemas["GenerateRequest"]["example"]["workspace"]
            .as_str()
            .unwrap_or_default();
        assert_eq!(ws, "demo", "示例值须与文档口径一致(workspace=demo)");
        let tag = schemas["GenerateRequest"]["example"]["biz_tag"]
            .as_str()
            .unwrap_or_default();
        assert_eq!(tag, "order", "示例值须与文档口径一致(biz_tag=order)");
    }
}
