// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

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
        // tag 体系与 handlers 注解载体的资源域对齐
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

// ============================================================================
// OpenAPI 运行时本地化（unify-rust-i18n）
// ============================================================================

/// 由 spec 路径生成 operation 键的 path-slug 段（与 etyma 先例同一规则）：
/// 去首尾 `/`，段间 `/` → `-`，剥离路径参数花括号
/// （`/api/v1/biz-tags/{id}` → `api-v1-biz-tags-id`，`/api/v1/` → `api-v1`）。
fn operation_slug(path: &str) -> String {
    path.trim_matches('/')
        .split('/')
        .map(|segment| segment.replace(['{', '}'], ""))
        .collect::<Vec<_>>()
        .join("-")
}

/// 是否 HTTP method 键（遍历 path item 成员时区分 method 与 `parameters` 等
/// OpenAPI 对象字段；口径与 FTL operation 键的 `<method>-` 前缀一致）。
fn is_http_method(s: &str) -> bool {
    matches!(
        s,
        "get" | "put" | "post" | "delete" | "options" | "head" | "patch" | "trace"
    )
}

/// 构建经运行时本地化的 OpenAPI JSON（`/api-docs/openapi.json` 出口）。
///
/// `ApiDoc` 的 utoipa derive 装配是编译期静态的，注解 description 只能是
/// 单语规范串（为英文规范串，与 FTL en 串逐字节一致）；此处序列化后
/// 按当前进程 locale 做「命中才替换」：
/// - response description 键 `<method>-<path-slug>-<status>`；
/// - parameter description 键 `<method>-<path-slug>-param-<name>`；
///
/// FTL 键登记见 `locales/{en,zh}/messages.ftl` 的 T021 段。未命中的
/// description 保持静态英文规范串，契约结构与 HTTP 语义不变。
pub(crate) fn localized_openapi_json() -> serde_json::Value {
    let mut doc = serde_json::to_value(ApiDoc::openapi()).expect("ApiDoc must serialize to JSON");
    localize_openapi_json(&mut doc, &crate::core::i18n::current_locale());
    doc
}

/// 就地替换 `doc["paths"]` 下各 operation 的 response/parameter description
/// （命中 FTL 键才替换，见 [`localized_openapi_json`]）。
fn localize_openapi_json(doc: &mut serde_json::Value, locale: &str) {
    let Some(paths) = doc.get_mut("paths").and_then(|p| p.as_object_mut()) else {
        return;
    };
    for (path, item) in paths.iter_mut() {
        let slug = operation_slug(path);
        let Some(item) = item.as_object_mut() else {
            continue;
        };
        for (method, op) in item.iter_mut() {
            if !is_http_method(method) {
                continue;
            }
            let base = format!("{method}-{slug}");
            let Some(op) = op.as_object_mut() else {
                continue;
            };
            // operation 级 description（当前注解未用；键命中即替换，供后续扩展）
            if let Some(text) = crate::core::i18n::translate_if_present(locale, &base) {
                op.insert("description".to_string(), serde_json::Value::String(text));
            }
            if let Some(responses) = op.get_mut("responses").and_then(|r| r.as_object_mut()) {
                for (status, response) in responses.iter_mut() {
                    let Some(text) = crate::core::i18n::translate_if_present(
                        locale,
                        &format!("{base}-{status}"),
                    ) else {
                        continue;
                    };
                    if let Some(response) = response.as_object_mut() {
                        response.insert("description".to_string(), serde_json::Value::String(text));
                    }
                }
            }
            if let Some(parameters) = op.get_mut("parameters").and_then(|p| p.as_array_mut()) {
                for parameter in parameters.iter_mut() {
                    let Some(parameter) = parameter.as_object_mut() else {
                        continue;
                    };
                    let Some(name) = parameter.get("name").and_then(|n| n.as_str()) else {
                        continue;
                    };
                    if let Some(text) = crate::core::i18n::translate_if_present(
                        locale,
                        &format!("{base}-param-{name}"),
                    ) {
                        parameter
                            .insert("description".to_string(), serde_json::Value::String(text));
                    }
                }
            }
        }
    }
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
    axum::Json(localized_openapi_json())
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

    // ========== —— paths 守卫 ==========

    /// `/api/v1` 业务路由清单—— openapi paths 与 `router.rs`
    /// 注册处的一一对应表（路径用 OpenAPI `{}` 语法；router.rs 注册处
    /// 为同名 `{}` 语法，语义一一对应；`/api/v1` 根以 `/api/v1/` 呈现）。
    ///
    /// 方案说明（选型见任务报告）： axum 0.8 `Router::routes()` 不展开
    /// `nest()` 的子路由，且完整装配 router 需要克隆整套 auth/audit/限流
    /// mock，故守卫不遍历运行时 Router，而是对齐既有 parity 清单
    /// （`router.rs::API_V1_ROUTE_PREFIXES` / api-info endpoints）维护本表：
    /// - 正向：本表每条路径+方法都必须出现在 openapi 文档（漏注解即失败）；
    /// - 反向：openapi 文档中 `/api/` 前缀路径不得超出本表（注解漂移即失败）；
    /// - 上界：paths 总数 ≥ 表长（任务要求的「paths 数 ≥ 路由数」）。
    /// 新增 /api/v1 路由时 守卫会先失败并把开发者引到 router.rs，
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
        // 统一错误信封：受保护端点的 4xx/5xx 都应引用 $ref ErrorResponse。
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
        // 核心四 schema 必须带 example（与 docs/API_REFERENCE.md 同口径）。
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

    // ========== —— OpenAPI 运行时本地化守卫 ==========

    /// operation_slug 单元测试：剥花括号、`/` → `-`、首尾 `/` 归零。
    #[test]
    fn operation_slug_strips_path_params_and_prefixes() {
        assert_eq!(
            operation_slug("/api/v1/biz-tags/{id}"),
            "api-v1-biz-tags-id"
        );
        assert_eq!(operation_slug("/api/v1/"), "api-v1");
        assert_eq!(operation_slug("/health"), "health");
        assert_eq!(
            operation_slug("/api/v1/workspaces/{name}/regenerate-user-key"),
            "api-v1-workspaces-name-regenerate-user-key"
        );
    }

    /// 从磁盘读取 en FTL 的键集合（与 `core::i18n` 键齐性守卫同一解析口径：
    /// `key = value` 行，`.`→`-` 映射不适用——本测试只消费 FTL 原生键）。
    fn en_ftl_keys() -> Vec<String> {
        let ftl = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales/en/messages.ftl"),
        )
        .expect("locales/en/messages.ftl must exist on disk");
        ftl.lines()
            .filter_map(|line| line.split_once(" = "))
            .map(|(key, _)| key.trim().to_string())
            .collect()
    }

    /// 守卫：messages.ftl 中全部 `<method>-` 前缀键都必须可由 spec 路由
    /// 经 `operation_slug` 计算命中（response 键 `<method>-<slug>-<status>`、
    /// parameter 键 `<method>-<slug>-param-<name>`）——防止 slug 与 FTL 键
    /// 漂移导致运行时替换静默失效。
    #[test]
    fn all_openapi_ftl_operation_keys_reachable_from_spec_routes() {
        let doc = openapi_json();
        let mut spec_keys: std::collections::HashSet<String> = Default::default();
        for (path, item) in doc["paths"].as_object().expect("paths must be an object") {
            let slug = operation_slug(path);
            for (method, op) in item.as_object().expect("path item must be an object") {
                if !is_http_method(method) {
                    continue;
                }
                let base = format!("{method}-{slug}");
                if let Some(responses) = op.get("responses").and_then(|r| r.as_object()) {
                    for status in responses.keys() {
                        spec_keys.insert(format!("{base}-{status}"));
                    }
                }
                if let Some(parameters) = op.get("parameters").and_then(|p| p.as_array()) {
                    for parameter in parameters {
                        if let Some(name) = parameter.get("name").and_then(|n| n.as_str()) {
                            spec_keys.insert(format!("{base}-param-{name}"));
                        }
                    }
                }
            }
        }

        let missing: Vec<String> = en_ftl_keys()
            .into_iter()
            .filter(|key| {
                is_http_method(key.split('-').next().unwrap_or_default())
                    && !spec_keys.contains(key)
            })
            .collect();
        assert!(
            missing.is_empty(),
            "FTL operation keys not reachable from spec routes (slug/键漂移?): {missing:?}"
        );
    }

    /// 本地化替换语义：en/zh 双 locale 下命中键的 description 均被
    /// 对应 FTL 文案替换（en 替换结果与注解内联英文规范串同源）。
    #[test]
    fn localized_openapi_replaces_descriptions_on_hit() {
        for locale in ["en", "zh-CN"] {
            let mut doc =
                serde_json::to_value(ApiDoc::openapi()).expect("ApiDoc must serialize to JSON");
            localize_openapi_json(&mut doc, locale);

            let generate = &doc["paths"]["/api/v1/generate"]["post"];
            let expected =
                crate::core::i18n::translate_if_present(locale, "post-api-v1-generate-200")
                    .expect("post-api-v1-generate-200 must hit the catalog");
            assert_eq!(
                generate["responses"]["200"]["description"].as_str(),
                Some(expected.as_str()),
                "200 description must be localized ({locale})"
            );

            // parameter description：GET /api/v1/groups 的 workspace 参数
            let groups = &doc["paths"]["/api/v1/groups"]["get"];
            let params = groups["parameters"].as_array().expect("parameters array");
            let workspace = params
                .iter()
                .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("workspace"))
                .expect("workspace parameter present");
            let expected = crate::core::i18n::translate_if_present(
                locale,
                "get-api-v1-groups-param-workspace",
            )
            .expect("get-api-v1-groups-param-workspace must hit the catalog");
            assert_eq!(
                workspace["description"].as_str(),
                Some(expected.as_str()),
                "parameter description must be localized ({locale})"
            );
        }
    }
}

#[cfg(test)]
mod localize_edge_shapes {
    //! `localize_openapi_json` 对畸形 spec 形状的容错分支：缺 paths、条目非
    //! 对象、非 HTTP 动词、responses/parameters 形状不符时必须安静跳过，
    //! 不得 panic，也不得产出错误的替换。

    use super::*;

    #[test]
    fn missing_paths_returns_unchanged() {
        let mut doc = serde_json::json!({ "info": { "title": "x" } });
        localize_openapi_json(&mut doc, "en");
        assert!(doc.get("paths").is_none(), "无 paths 时文档必须原样保留");
    }

    #[test]
    fn malformed_path_items_and_methods_are_skipped() {
        let mut doc = serde_json::json!({
            "paths": {
                // path 条目非对象 → continue
                "/api/v1/x": "not-an-object",
                // operation 条目非对象 → continue；非 HTTP 动词 → continue
                "/api/v1/y": { "get": "not-an-object", "trace": { "responses": {} } },
            }
        });
        localize_openapi_json(&mut doc, "en");
        assert_eq!(doc["paths"]["/api/v1/x"], "not-an-object");
        assert_eq!(doc["paths"]["/api/v1/y"]["get"], "not-an-object");
    }

    #[test]
    fn malformed_responses_and_parameters_are_skipped() {
        let mut doc = serde_json::json!({
            "paths": {
                // responses 非 object / parameters 非 array → 内层循环跳过
                "/api/v1/z": { "get": {
                    "responses": "not-an-object",
                    "parameters": "not-an-array",
                } },
                // response 条目非 object → continue；parameter 缺 name → continue
                "/api/v1/w": { "get": {
                    "responses": { "500": "not-an-object" },
                    "parameters": [ { "in": "query" } ],
                } },
            }
        });
        localize_openapi_json(&mut doc, "en");
        assert_eq!(
            doc["paths"]["/api/v1/z"]["get"]["responses"],
            "not-an-object"
        );
        assert_eq!(
            doc["paths"]["/api/v1/w"]["get"]["responses"]["500"],
            "not-an-object"
        );
    }
}
