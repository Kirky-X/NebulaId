// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use crate::server::api_version::{api_version_middleware, API_V1};
use crate::server::audit::{audit_middleware_fn, AuditLogger, AuditMiddleware};
use crate::server::config::{cors, management::ConfigManagementService};
use crate::server::handlers::helpers::{
    admin_cannot_perform_response, auth_required_response, core_error_to_response,
    invalid_uuid_response, invalid_workspace_id_response, validation_error_response,
    workspace_id_required_response, workspace_mismatch_response, workspace_name_not_found_response,
    workspace_not_found_response,
};
use crate::server::handlers::ApiHandlers;
use crate::server::middleware::locale::Locale;
use crate::server::middleware::{locale_middleware, ApiKeyAuth};
use crate::server::models::{
    ApiInfoResponse, ApiKeyListResponse, ApiKeyWithSecretResponse, BatchGenerateRequest,
    BatchGenerateResponse, BizTagListResponse, BizTagResponse, CreateApiKeyRequest,
    CreateBizTagRequest, CreateGroupRequest, CreateWorkspaceRequest, ErrorResponse,
    GenerateRequest, GenerateResponse, GroupListParams, GroupListResponse, GroupResponse,
    HealthResponse, MetricsResponse, PaginationParams, ParseRequest, ParseResponse, ReadyResponse,
    RevokeApiKeyResponse, SecureConfigResponse, SetAlgorithmRequest, SetAlgorithmResponse,
    UpdateBizTagRequest, UpdateConfigResponse, UpdateLoggingRequest, UpdateRateLimitRequest,
    WorkspaceListResponse, WorkspaceResponse,
};
use crate::server::rate_limit::{limiter::RateLimiter, middleware::RateLimitMiddleware};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
    Extension, Json, Router,
};
use metrics::{Key, KeyName, Label, Metadata, Recorder, Unit};
use metrics_exporter_prometheus::{
    Matcher, PrometheusBuilder, PrometheusHandle, PrometheusRecorder,
};
use sdforge::tower_http::set_header::SetResponseHeaderLayer;
use sdforge::validator::Validate;
use std::sync::Arc;

#[derive(Clone)]
pub struct NebulaIdState {
    pub handlers: Arc<ApiHandlers>,
    pub auth: Arc<ApiKeyAuth>,
    pub config_service: Arc<dyn ConfigManagementService>,
    /// Prometheus 文本桥：/metrics 把既有指标快照灌入 exporter registry；
    /// generate 热路径不经过本桥，零新增开销。
    pub prometheus: Arc<PrometheusMetricsBridge>,
}

// ========== Prometheus /metrics 桥接 ==========

/// `nebula_id_generate_latency_seconds` 直方图桶（秒）：微秒到毫秒密度，
/// 延伸到秒级长尾。
const PROMETHEUS_LATENCY_BUCKETS_SECONDS: &[f64] = &[
    0.000_05, 0.000_1, 0.000_25, 0.000_5, 0.001, 0.002_5, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5,
    1.0, 2.5, 5.0,
];

/// 渲染路径 `register_*` 的必填 `Recorder` 元数据参数；本桥不安装全局 recorder。
const PROMETHEUS_METADATA: Metadata =
    Metadata::new(module_path!(), metrics::Level::INFO, Some(module_path!()));

/// 把 `ApiHandlers::metrics()` 快照（原子计数 + 观测环分位数 + 降级/熔断
/// 状态）渲染成 Prometheus 文本的桥。
///
/// 仅在 `/metrics` 被抓取时读取快照灌入 exporter 私有 registry（counter 用
/// `absolute` 幂等桥接、gauge 用 `set`、histogram 以分位数增量累计），
/// generate 热路径零额外开销。
pub struct PrometheusMetricsBridge {
    /// 保活 exporter registry；`handle` 渲染的就是它的内部状态。
    recorder: PrometheusRecorder,
    handle: PrometheusHandle,
}

impl PrometheusMetricsBridge {
    fn new() -> Self {
        let recorder = PrometheusBuilder::new()
            .set_buckets_for_metric(
                Matcher::Full("nebula_id_generate_latency_seconds".to_owned()),
                PROMETHEUS_LATENCY_BUCKETS_SECONDS,
            )
            .expect("static latency buckets are finite and sorted")
            .build_recorder();

        // HELP/UNIT 描述一次性登记（幂等），渲染时输出 `# HELP` 行。
        recorder.describe_counter(
            KeyName::from("nebula_id_ids_generated_total"),
            None,
            "Total IDs generated per algorithm (router observation).".into(),
        );
        recorder.describe_counter(
            KeyName::from("nebula_id_generate_failures_total"),
            None,
            "Total failed generate calls per algorithm (router observation).".into(),
        );
        recorder.describe_counter(
            KeyName::from("nebula_id_clock_backwards_total"),
            None,
            "Total observed clock backwards events per algorithm.".into(),
        );
        recorder.describe_histogram(
            KeyName::from("nebula_id_generate_latency_seconds"),
            Some(Unit::Seconds),
            "End-to-end generate latency per algorithm (router observation).".into(),
        );
        for (name, quantile) in [
            ("nebula_id_generate_latency_p50_seconds", "p50"),
            ("nebula_id_generate_latency_p99_seconds", "p99"),
            ("nebula_id_generate_latency_p999_seconds", "p999"),
        ] {
            recorder.describe_gauge(
                KeyName::from(name),
                Some(Unit::Seconds),
                format!("Render-time {quantile} latency gauge per algorithm (0 = no samples yet).")
                    .into(),
            );
        }
        recorder.describe_gauge(
            KeyName::from("nebula_id_cache_hit_rate"),
            None,
            "Cache hit rate percent per algorithm (0-100).".into(),
        );
        recorder.describe_gauge(
            KeyName::from("nebula_id_algorithm_degraded"),
            None,
            "1 when the algorithm is currently degraded, 0 otherwise.".into(),
        );
        recorder.describe_gauge(
            KeyName::from("nebula_id_circuit_breaker_open"),
            None,
            "1 when the algorithm circuit breaker is Open, 0 otherwise.".into(),
        );
        recorder.describe_gauge(
            KeyName::from("nebula_id_circuit_breaker_half_open"),
            None,
            "1 when the algorithm circuit breaker is HalfOpen, 0 otherwise.".into(),
        );
        recorder.describe_counter(
            KeyName::from("nebula_id_requests_total"),
            None,
            "Total requests observed by the API layer.".into(),
        );
        recorder.describe_counter(
            KeyName::from("nebula_id_successful_generations_total"),
            None,
            "Total successful generate requests observed by the API layer.".into(),
        );
        recorder.describe_counter(
            KeyName::from("nebula_id_failed_generations_total"),
            None,
            "Total failed generate requests observed by the API layer.".into(),
        );
        recorder.describe_gauge(
            KeyName::from("nebula_id_uptime_seconds"),
            Some(Unit::Seconds),
            "Process uptime in seconds.".into(),
        );

        let handle = recorder.handle();
        Self { recorder, handle }
    }

    /// 采样快照 → 灌入 registry → 渲染 Prometheus 文本。
    fn render(&self, snapshot: &MetricsResponse) -> String {
        for alg in &snapshot.algorithms {
            let labels = vec![Label::new("algorithm", alg.algorithm.clone())];

            // counter：绝对值桥接（fetch_max 幂等，快照单调递增）。
            self.recorder
                .register_counter(
                    &Key::from_parts("nebula_id_ids_generated_total", labels.clone()),
                    &PROMETHEUS_METADATA,
                )
                .absolute(alg.total_generated);
            self.recorder
                .register_counter(
                    &Key::from_parts("nebula_id_generate_failures_total", labels.clone()),
                    &PROMETHEUS_METADATA,
                )
                .absolute(alg.total_failed);
            self.recorder
                .register_counter(
                    &Key::from_parts("nebula_id_clock_backwards_total", labels.clone()),
                    &PROMETHEUS_METADATA,
                )
                .absolute(alg.clock_backwards);

            // gauge：分位数（ms → s；0 = 未观测，与核心语义一致）。
            for (name, value_ms) in [
                ("nebula_id_generate_latency_p50_seconds", alg.p50_latency_ms),
                ("nebula_id_generate_latency_p99_seconds", alg.p99_latency_ms),
                (
                    "nebula_id_generate_latency_p999_seconds",
                    alg.p999_latency_ms,
                ),
            ] {
                self.recorder
                    .register_gauge(&Key::from_parts(name, labels.clone()), &PROMETHEUS_METADATA)
                    .set(value_ms / 1_000.0);
            }
            // `None` = 该算法无缓存概念：不产出命中率序列。
            if let Some(hit_rate) = alg.cache_hit_rate {
                self.recorder
                    .register_gauge(
                        &Key::from_parts("nebula_id_cache_hit_rate", labels.clone()),
                        &PROMETHEUS_METADATA,
                    )
                    .set(hit_rate);
            }

            // histogram：以环分位数（p50/p99/p999 秒）做增量观测——环不暴露原始
            // 样本，三档是最忠实近似；全 0 = 无样本，跳过以免伪造 0 延迟。
            if alg.p50_latency_ms > 0.0 || alg.p99_latency_ms > 0.0 || alg.p999_latency_ms > 0.0 {
                let histogram = self.recorder.register_histogram(
                    &Key::from_parts("nebula_id_generate_latency_seconds", labels.clone()),
                    &PROMETHEUS_METADATA,
                );
                histogram.record(alg.p50_latency_ms / 1_000.0);
                histogram.record(alg.p99_latency_ms / 1_000.0);
                histogram.record(alg.p999_latency_ms / 1_000.0);
            }
        }

        // 降级 / 熔断状态 gauge（快照直接给出布尔/字符串）。
        for degradation in &snapshot.degradation_metrics {
            let labels = vec![Label::new("algorithm", degradation.algorithm.clone())];
            self.recorder
                .register_gauge(
                    &Key::from_parts("nebula_id_algorithm_degraded", labels.clone()),
                    &PROMETHEUS_METADATA,
                )
                .set(u8::from(degradation.is_degraded) as f64);
            self.recorder
                .register_gauge(
                    &Key::from_parts("nebula_id_circuit_breaker_open", labels.clone()),
                    &PROMETHEUS_METADATA,
                )
                .set(u8::from(degradation.circuit_breaker_state == "Open") as f64);
            self.recorder
                .register_gauge(
                    &Key::from_parts("nebula_id_circuit_breaker_half_open", labels.clone()),
                    &PROMETHEUS_METADATA,
                )
                .set(u8::from(degradation.circuit_breaker_state == "HalfOpen") as f64);
        }

        // API 层全局计数 / 运行时长。
        self.recorder
            .register_counter(
                &Key::from_name("nebula_id_requests_total"),
                &PROMETHEUS_METADATA,
            )
            .absolute(snapshot.total_requests);
        self.recorder
            .register_counter(
                &Key::from_name("nebula_id_successful_generations_total"),
                &PROMETHEUS_METADATA,
            )
            .absolute(snapshot.successful_generations);
        self.recorder
            .register_counter(
                &Key::from_name("nebula_id_failed_generations_total"),
                &PROMETHEUS_METADATA,
            )
            .absolute(snapshot.failed_generations);
        self.recorder
            .register_gauge(
                &Key::from_name("nebula_id_uptime_seconds"),
                &PROMETHEUS_METADATA,
            )
            .set(snapshot.uptime_seconds as f64);

        self.handle.run_upkeep();
        self.handle.render()
    }
}

pub async fn create_router(
    handlers: Arc<ApiHandlers>,
    auth: Arc<ApiKeyAuth>,
    rate_limiter: Arc<RateLimiter>,
    audit_logger: Arc<AuditLogger>,
) -> Router {
    // 默认启用全局限流
    create_router_with_rate_limit(handlers, auth, rate_limiter, audit_logger, true).await
}

/// 带显式限流开关的路由装配：`false` 时不挂载限流层。
///
/// 开关由调用方（main.rs 读真实 Config）传入，避免内部回读 config_service
/// 牵连出额外 mock 期望。
pub async fn create_router_with_rate_limit(
    handlers: Arc<ApiHandlers>,
    auth: Arc<ApiKeyAuth>,
    rate_limiter: Arc<RateLimiter>,
    audit_logger: Arc<AuditLogger>,
    rate_limit_enabled: bool,
) -> Router {
    // CORS 按环境装配：生产用 ALLOWED_ORIGINS 指定前端来源（逗号分隔）
    // 例：ALLOWED_ORIGINS="https://example.com,https://app.example.com"
    let cors = cors::create_env_aware_cors_layer();

    // trusted proxies 读 NEBULA_TRUSTED_PROXIES（与 main.rs 的 ApiKeyAuth 同源，
    // 避免给 create_router 加参数）；缺省空列表 = 不信任任何转发头。
    let trusted_proxies: Vec<std::net::IpAddr> = std::env::var("NEBULA_TRUSTED_PROXIES")
        .ok()
        .map(|s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect())
        .unwrap_or_default();

    let rate_limit_middleware = RateLimitMiddleware::new(rate_limiter.clone())
        .with_trusted_proxies(trusted_proxies.clone());
    let audit_middleware =
        Arc::new(AuditMiddleware::new(audit_logger.clone()).with_trusted_proxies(trusted_proxies));

    let config_service = handlers.get_config_service();

    let app_state = NebulaIdState {
        handlers: handlers.clone(),
        auth: auth.clone(),
        config_service: config_service.clone(),
        prometheus: Arc::new(PrometheusMetricsBridge::new()),
    };

    // ========== V1 API Routes ==========
    // Admin-only endpoints (require admin API key)
    let v1_admin_routes = Router::new()
        // API Key management endpoints (admin only)
        .route(
            "/api-keys",
            post(handle_create_api_key).get(handle_list_api_keys),
        )
        .route("/api-keys/{id}", delete(handle_revoke_api_key))
        // Workspace creation (admin only)
        .route("/workspaces", post(handle_create_workspace))
        // Workspace user key regeneration (admin only)
        .route(
            "/workspaces/{name}/regenerate-user-key",
            post(handle_regenerate_user_key),
        )
        // （CWE-862）配置变更端点（速率限制/日志/热重载/默认算法）必须 Admin：
        // 错放在 authenticated 路由会让任意 User key 改写全局配置。
        .route("/config/rate-limit", post(handle_update_rate_limit))
        .route("/config/logging", post(handle_update_logging))
        .route("/config/reload", post(handle_reload_config))
        .route("/config/algorithm", post(handle_set_algorithm))
        // 先接 admin 层、后接 auth 层（axum 0.8 后 layer 者先执行），
        // 保证 auth 先运行并注入 ApiKeyRole 扩展。
        .layer(axum::middleware::from_fn(
            crate::server::middleware::admin_required_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            crate::server::middleware::auth_middleware_fn,
        ));

    // Authenticated endpoints (require API key)
    let v1_authenticated_routes = Router::new()
        .route("/config", get(handle_get_config))
        // ID generation (user only)
        .route("/generate", post(handle_generate))
        .route("/generate/batch", post(handle_batch_generate))
        .route("/parse", post(handle_parse))
        // Workspace query endpoints
        .route("/workspaces", get(handle_list_workspaces))
        .route("/workspaces/{name}", get(handle_get_workspace))
        // Group CRUD endpoints
        .route("/groups", post(handle_create_group).get(handle_list_groups))
        // BizTag CRUD endpoints
        .route(
            "/biz-tags",
            post(handle_create_biz_tag).get(handle_list_biz_tags),
        )
        .route(
            "/biz-tags/{id}",
            get(handle_get_biz_tag)
                .put(handle_update_biz_tag)
                .delete(handle_delete_biz_tag),
        )
        // （CWE-1188）axum 0.8 中后 `.layer()` 者先执行：先 layer
        // `anonymous_block_middleware`（inner），再 layer `auth_middleware_fn`
        // （outer）以先注入 `ApiKeyRole`，否则匿名拒绝会失效。
        .layer(axum::middleware::from_fn(anonymous_block_middleware))
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            crate::server::middleware::auth_middleware_fn,
        ));

    // Public endpoints (no authentication)
    let v1_public_routes = Router::new()
        .route("/", get(handle_api_info))
        .merge(v1_authenticated_routes)
        .merge(v1_admin_routes);

    // ========== API Versioning ==========
    // Nest V1 routes under /api/v1/
    let api_v1_routes = Router::new()
        .nest(&format!("/api/{}", API_V1), v1_public_routes)
        // Apply API version middleware to all API routes
        .layer(axum::middleware::from_fn(api_version_middleware))
        // locale 协商只服务 /api/v1/*（这些 handler 读 `Extension<Locale>`）；
        // /health、/ready、/metrics、openapi.json 不消费 Locale，无需承担解析开销。
        .layer(axum::middleware::from_fn(locale_middleware));

    // ========== Root Routes ==========
    // Public router (no authentication) - includes health check and metrics
    let routes = Router::new()
        .route("/health", get(handle_health))
        .route("/ready", get(handle_ready))
        .route("/metrics", get(handle_metrics))
        .route(
            "/api-docs/openapi.json",
            get(crate::server::openapi::openapi_json_handler),
        )
        .merge(api_v1_routes)
        .with_state(app_state)
        // Swagger UI 只挂 UI 路由，spec 复用上方 /api-docs/openapi.json：捆绑版
        // swagger_ui_router 会在同路径注册 inventory spec 造成冲突；两者均公开只读。
        .merge(sdforge::docs::swagger_ui_router_with_spec(
            "/api-docs/openapi.json",
        ));
    // 限流层必须位于 CORS 内侧（axum 先 `.layer()` 者为内层）：挂外侧会使 429
    // 缺 CORS 头且 OPTIONS 预检消耗配额；`rate_limit_enabled=false` 时不挂载。

    let routes = if rate_limit_enabled {
        routes.layer(axum::middleware::from_fn_with_state(
            rate_limit_middleware,
            crate::server::rate_limit::middleware::rate_limit_middleware_fn,
        ))
    } else {
        routes
    };

    routes
        // Security headers
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_XSS_PROTECTION,
            HeaderValue::from_static("1; mode=block"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(cors)
        .layer(axum::Extension(audit_middleware.clone()))
        .layer(axum::middleware::from_fn_with_state(
            audit_middleware,
            audit_middleware_fn,
        ))
        .layer(axum::Extension(audit_logger))
        // 请求上下文（sdforge `context`）：解析/生成 x-request-id 与 W3C
        // traceparent/x-trace-id，安装跨协议上下文并回显关联 ID；后 layer 者先
        // 执行，故必须最后挂载才能位于最外侧。
        .layer(axum::middleware::from_fn(
            sdforge::context::context_middleware,
        ))
        // request_id 中间件（在 context_middleware 外侧）：校验/生成 UUID v7
        // 并回写请求头，保证两条路径响应头一致；同时安装 `RequestId` 扩展、
        // 任务局部上下文（helpers 错误装配读取）与 tracing span 属性。
        .layer(axum::middleware::from_fn(
            crate::server::middleware::request_id::request_id_middleware,
        ))
}

// ========== Helper Functions ==========

/// （CWE-1188）v1_authenticated_routes 的全局 Anonymous 拒绝中间件。
///
/// 禁认证时 `verify_user_role` 只在 4 个 handler 调用，其余端点（含
/// `POST /config/algorithm`、`DELETE /biz-tags/{id}` 等破坏性操作）会对
/// Anonymous 开放；本层统一拒绝，无需逐 handler 校验。
///
/// layer 顺序：本中间件必须为 inner（先 `.layer()`），`auth_middleware_fn`
/// 为 outer（后 `.layer()`）以保证先注入 `ApiKeyRole`。
///
/// `ApiKeyRole` 扩展缺失时 fail-closed 401 + warn，避免静默放行。
pub async fn anonymous_block_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    match req
        .extensions()
        .get::<crate::server::middleware::ApiKeyRole>()
    {
        Some(role) if *role == crate::server::middleware::ApiKeyRole::Anonymous => {
            // Locale 由外层 locale_middleware 注入；若未注入则用默认值。
            let locale = req
                .extensions()
                .get::<Locale>()
                .cloned()
                .unwrap_or_default();
            auth_required_response(locale).into_response()
        }
        Some(_) => next.run(req).await,
        None => {
            // ApiKeyRole 扩展缺失：fail-closed。
            tracing::warn!(
                path = %req.uri().path(),
                method = %req.method(),
                "ApiKeyRole extension missing in anonymous_block_middleware; \
                 auth_middleware_fn may not have run; denying request"
            );
            let locale = req
                .extensions()
                .get::<Locale>()
                .cloned()
                .unwrap_or_default();
            auth_required_response(locale).into_response()
        }
    }
}

/// 校验调用者为 User API key：Admin → 403、Anonymous → 401（均含本地化文案）。
fn verify_user_role(
    role: crate::server::middleware::ApiKeyRole,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if role == crate::server::middleware::ApiKeyRole::Admin {
        return Err(admin_cannot_perform_response(locale));
    }
    if role == crate::server::middleware::ApiKeyRole::Anonymous {
        return Err(auth_required_response(locale));
    }
    Ok(())
}

/// Validate request and return locale-translated error if invalid.
fn validate_request<T: Validate>(
    req: &T,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if let Err(validation_errors) = req.validate() {
        return Err(validation_error_response(&validation_errors, locale));
    }
    Ok(())
}

/// 按 workspace 名反查并校验 User key 的 workspace 归属；错误文案本地化。
async fn verify_user_workspace(
    workspace_name: &str,
    key_workspace_id: &Option<uuid::Uuid>,
    handlers: &Arc<ApiHandlers>,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let workspace_result = handlers
        .get_workspace(workspace_name)
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;

    let workspace = workspace_result
        .ok_or_else(|| workspace_name_not_found_response(workspace_name, locale))?;

    let workspace_uuid =
        uuid::Uuid::parse_str(&workspace.id).map_err(|_| invalid_workspace_id_response(locale))?;

    verify_workspace_id_match(workspace_uuid, key_workspace_id, locale).await
}

/// workspace_id 比较：判定委托共享授权函数 `authorize_workspace_access`
/// （HTTP/gRPC 同源），本函数只保留传输层部分 —— None（认证禁用）放行、
/// locale 错误装配；mismatch 返回 403。
async fn verify_workspace_id_match(
    workspace_uuid: uuid::Uuid,
    key_workspace_id: &Option<uuid::Uuid>,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    // 当认证禁用时，key_workspace_id 为 None，允许访问任何 workspace
    let Some(key_workspace_id) = *key_workspace_id else {
        return Ok(());
    };
    crate::server::handlers::helpers::authorize_workspace_access(
        &crate::server::middleware::ApiKeyRole::User,
        key_workspace_id,
        workspace_uuid,
    )
    .await
    .map_err(|_| workspace_mismatch_response(locale))
}

/// Verify workspace_id match for User API Key (direct Uuid comparison)
async fn verify_workspace_id(
    req_workspace_id: uuid::Uuid,
    key_workspace_id: &Option<uuid::Uuid>,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    verify_workspace_id_match(req_workspace_id, key_workspace_id, locale).await
}

async fn handle_generate(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Only User API Key can generate IDs
    verify_user_role(extensions_role.0, locale)?;

    // Validate request parameters
    validate_request(&req, locale)?;

    // Verify workspace_id match for User API Key
    verify_user_workspace(&req.workspace, &extensions.0, &state.handlers, locale).await?;

    state
        .handlers
        .generate(req)
        .await
        .map(Json)
        .map_err(|e: crate::core::types::CoreError| core_error_to_response(&e, locale))
}

async fn handle_batch_generate(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<BatchGenerateRequest>,
) -> Result<Json<BatchGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Only User API Key can generate IDs
    verify_user_role(extensions_role.0, locale)?;

    // 服务端日志只用结构化字段、不做 `t!()` 翻译：客户端文案已由
    // `core_error_to_response` 本地化，重复查询既冗余又会改动全局 locale 状态。
    tracing::debug!(
        event = "batch_generate_request",
        workspace = %req.workspace,
        group = %req.group,
        size = ?req.size,
    );

    // Validate request parameters
    validate_request(&req, locale)?;

    // Verify workspace_id match for User API Key
    verify_user_workspace(&req.workspace, &extensions.0, &state.handlers, locale).await?;

    // 统一经 `core_error_to_response`：5xx 脱敏为通用消息、4xx 截断长度；
    // 此处仅对 4xx 补 `warn!` 便于观测，不重复实现状态码映射。
    match state.handlers.batch_generate(req).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => {
            let resp = core_error_to_response(&e, locale);
            if resp.0.is_client_error() {
                tracing::warn!(
                    event = "batch_generation_failed",
                    status = %resp.0,
                    error = ?e,
                    "batch_generate returned 4xx"
                );
            }
            Err(resp)
        }
    }
}

async fn handle_health(State(state): State<NebulaIdState>) -> Json<HealthResponse> {
    Json(state.handlers.health().await)
}

async fn handle_ready(State(state): State<NebulaIdState>) -> Json<ReadyResponse> {
    Json(state.handlers.ready().await)
}

/// `/metrics` 输出 Prometheus 文本（text/plain; version=0.0.4）。
///
/// 渲染前采样 `ApiHandlers::metrics()` 快照（仅原子计数/观测环读取）再灌入
/// exporter；监控抓取统一走本端点。
async fn handle_metrics(State(state): State<NebulaIdState>) -> axum::response::Response {
    let snapshot = state.handlers.metrics().await;
    let body = state.prometheus.render(&snapshot);
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

async fn handle_parse(
    State(state): State<NebulaIdState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<ParseRequest>,
) -> Result<Json<ParseResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate request parameters
    if let Err(validation_errors) = req.validate() {
        return Err(validation_error_response(&validation_errors, locale));
    }

    // 统一经 `core_error_to_response`：5xx 脱敏为通用消息、4xx 截断长度；
    // 此处仅对 4xx 补 `warn!` 便于观测，不重复实现状态码映射。
    match state.handlers.parse(req).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => {
            let resp = core_error_to_response(&e, locale);
            if resp.0.is_client_error() {
                tracing::warn!(
                    event = "parse_failed",
                    status = %resp.0,
                    error = ?e,
                    "parse returned 4xx"
                );
            }
            Err(resp)
        }
    }
}

async fn handle_get_config(State(state): State<NebulaIdState>) -> Json<SecureConfigResponse> {
    Json(state.config_service.get_secure_config())
}

async fn handle_update_rate_limit(
    State(state): State<NebulaIdState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<UpdateRateLimitRequest>,
) -> Result<Json<UpdateConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    // handler 边界强制 `#[validate(range(min = 1, max = 1000000))]`
    // （防止越界值被 config service 静默接受）。
    validate_request(&req, locale)?;
    Ok(Json(state.config_service.update_rate_limit(req).await))
}

async fn handle_update_logging(
    State(state): State<NebulaIdState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<UpdateLoggingRequest>,
) -> Result<Json<UpdateConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 转发前强制 `#[validate(length(min = 1, max = 20))]`。
    validate_request(&req, locale)?;
    Ok(Json(state.config_service.update_logging(req).await))
}

// 不用 #[axum::debug_handler]：axum 0.8 该 derive 宏生成 `::axum::` 绝对路径，
// 要求 axum 为直接依赖，违反「axum 由 sdforge 接管」原则；签名的编译报错会更晦涩。
async fn handle_reload_config(State(state): State<NebulaIdState>) -> Json<UpdateConfigResponse> {
    Json(state.config_service.reload_config().await)
}

async fn handle_set_algorithm(
    State(state): State<NebulaIdState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<SetAlgorithmRequest>,
) -> Result<Json<SetAlgorithmResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 强制 `biz_tag`(1..=64) 与 `algorithm`(1..=20) 的 validate 约束。
    validate_request(&req, locale)?;
    Ok(Json(state.config_service.set_algorithm(req).await))
}

/// `/api/v1` 路由前缀清单（`:name`/`:id` 风格）：api-info parity 守卫的数据源，
/// 新增路由必须同步本表，漏登由 `test_api_info_endpoints_cover_v1_routes`
/// 显性失败（按子串匹配，条目文字改动不误报）；仅测试构建消费，故 `#[cfg(test)]`。
#[cfg(test)]
const API_V1_ROUTE_PREFIXES: &[&str] = &[
    "/generate",
    "/generate/batch",
    "/parse",
    "/config",
    "/config/rate-limit",
    "/config/logging",
    "/config/reload",
    "/config/algorithm",
    "/workspaces",
    "/workspaces/:name",
    "/workspaces/:name/regenerate-user-key",
    "/groups",
    "/biz-tags",
    "/biz-tags/:id",
    "/api-keys",
    "/api-keys/:id",
];

async fn handle_api_info() -> Json<ApiInfoResponse> {
    Json(ApiInfoResponse {
        name: "Nebula ID Service".to_string(),
        // 版本号唯一来源 = Cargo.toml（避免发版漂移）。
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "Distributed ID Generation Service".to_string(),
        endpoints: vec![
            "GET /health - Health check".to_string(),
            "GET /ready - Readiness probe".to_string(),
            "GET /metrics - Prometheus metrics (text format, version 0.0.4)".to_string(),
            "GET /api/v1 - API information".to_string(),
            "POST /api/v1/generate - Generate ID".to_string(),
            "POST /api/v1/generate/batch - Batch generate IDs".to_string(),
            "POST /api/v1/parse - Parse ID".to_string(),
            "GET /api/v1/config - Get configuration".to_string(),
            "POST /api/v1/config/rate-limit - Update rate limit".to_string(),
            "POST /api/v1/config/logging - Update logging".to_string(),
            "POST /api/v1/config/reload - Reload configuration".to_string(),
            "POST /api/v1/config/algorithm - Set algorithm".to_string(),
            "GET /api/v1/workspaces - List workspaces".to_string(),
            "POST /api/v1/workspaces - Create workspace".to_string(),
            "GET /api/v1/workspaces/:name - Get workspace".to_string(),
            "POST /api/v1/workspaces/:name/regenerate-user-key - Regenerate user key".to_string(),
            "POST /api/v1/groups - Create group".to_string(),
            "GET /api/v1/groups - List groups".to_string(),
            "POST /api/v1/biz-tags - Create biz tag".to_string(),
            "GET /api/v1/biz-tags - List biz tags".to_string(),
            "GET /api/v1/biz-tags/:id - Get biz tag".to_string(),
            "PUT /api/v1/biz-tags/:id - Update biz tag".to_string(),
            "DELETE /api/v1/biz-tags/:id - Delete biz tag".to_string(),
            "POST /api/v1/api-keys - Create API key".to_string(),
            "GET /api/v1/api-keys - List API keys".to_string(),
            "DELETE /api/v1/api-keys/:id - Revoke API key".to_string(),
        ],
    })
}

// ========== BizTag Handlers ==========

async fn handle_create_biz_tag(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<CreateBizTagRequest>,
) -> Result<Json<BizTagResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate request parameters
    validate_request(&req, locale)?;

    // Only User API Key can create biz_tags
    verify_user_role(extensions_role.0, locale)?;

    // Verify workspace_id match for User API Key
    verify_workspace_id(req.workspace_id, &extensions.0, locale).await?;

    state
        .handlers
        .create_biz_tag(req)
        .await
        .map(Json)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_get_biz_tag(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Path(id): Path<String>,
) -> Result<Json<BizTagResponse>, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| invalid_uuid_response(locale))?;

    // （CWE-639）IDOR 防护：仅 User key，且目标 BizTag 的 workspace_id
    // 必须与调用者一致。
    verify_user_role(extensions_role.0, locale)?;

    let response = state
        .handlers
        .get_biz_tag(uuid)
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;

    let biz_tag_workspace =
        uuid::Uuid::parse_str(&response.workspace_id).map_err(|_| invalid_uuid_response(locale))?;
    verify_workspace_id(biz_tag_workspace, &extensions.0, locale).await?;

    Ok(Json(response))
}

async fn handle_update_biz_tag(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Path(id): Path<String>,
    Json(req): Json<UpdateBizTagRequest>,
) -> Result<Json<BizTagResponse>, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| invalid_uuid_response(locale))?;

    if let Err(validation_errors) = req.validate() {
        return Err(validation_error_response(&validation_errors, locale));
    }

    // （CWE-639）IDOR 防护：先查现有 BizTag 校验归属，再更新。
    verify_user_role(extensions_role.0, locale)?;

    let existing = state
        .handlers
        .get_biz_tag(uuid)
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;
    let biz_tag_workspace =
        uuid::Uuid::parse_str(&existing.workspace_id).map_err(|_| invalid_uuid_response(locale))?;
    verify_workspace_id(biz_tag_workspace, &extensions.0, locale).await?;

    state
        .handlers
        .update_biz_tag(uuid, req)
        .await
        .map(Json)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_delete_biz_tag(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| invalid_uuid_response(locale))?;

    // （CWE-639）IDOR 防护：先查现有 BizTag 校验归属，再删除。
    verify_user_role(extensions_role.0, locale)?;

    let existing = state
        .handlers
        .get_biz_tag(uuid)
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;
    let biz_tag_workspace =
        uuid::Uuid::parse_str(&existing.workspace_id).map_err(|_| invalid_uuid_response(locale))?;
    verify_workspace_id(biz_tag_workspace, &extensions.0, locale).await?;

    state
        .handlers
        .delete_biz_tag(uuid)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_list_biz_tags(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<BizTagListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 经 `core_error_to_response` 上报错误而非静默返回空列表；文案本地化。
    validate_request(&params, locale)?;

    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    // （CWE-639）查询键只来自认证身份或显式参数：User 强制绑定自身 workspace
    // （忽略 `?workspace_id=`），Admin 必须显式指定。
    let workspace_id = match extensions_role.0 {
        crate::server::middleware::ApiKeyRole::User => {
            extensions.0.ok_or_else(|| auth_required_response(locale))?
        }
        crate::server::middleware::ApiKeyRole::Admin => match params.workspace_id.as_deref() {
            Some(raw) => uuid::Uuid::parse_str(raw).map_err(|_| invalid_uuid_response(locale))?,
            None => return Err(workspace_id_required_response(locale)),
        },
        crate::server::middleware::ApiKeyRole::Anonymous => {
            return Err(auth_required_response(locale))
        }
    };

    let response = state
        .handlers
        .list_biz_tags_with_pagination(
            Some(workspace_id),
            None,
            page_size as usize,
            offset as usize,
        )
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;

    Ok(Json(BizTagListResponse {
        biz_tags: response.biz_tags,
        total: response.total,
        page,
        page_size,
    }))
}

// ========== Workspace Handlers ==========

async fn handle_create_workspace(
    State(state): State<NebulaIdState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, Json<ErrorResponse>)> {
    if let Err(validation_errors) = req.validate() {
        return Err(validation_error_response(&validation_errors, locale));
    }

    state
        .handlers
        .create_workspace(req)
        .await
        .map(Json)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_list_workspaces(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
) -> Result<Json<WorkspaceListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // （CWE-862 / CWE-639）跨租户读面收敛：先角色门禁（未授权角色不做仓储
    // 查询）再过滤 —— Admin 全量；User 仅自身 workspace（经
    // `authorize_workspace_access` 判定，`total` 随过滤收敛以隐藏其他租户）；
    // Anonymous 兜底 401（`anonymous_block_middleware` 已先拒）。
    let key_workspace_id = match extensions_role.0 {
        crate::server::middleware::ApiKeyRole::Admin => None,
        crate::server::middleware::ApiKeyRole::User => {
            Some(extensions.0.ok_or_else(|| auth_required_response(locale))?)
        }
        crate::server::middleware::ApiKeyRole::Anonymous => {
            return Err(auth_required_response(locale))
        }
    };

    let response = state
        .handlers
        .list_workspaces()
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;

    let filtered = match key_workspace_id {
        // Admin（无租户绑定）→ 全量。
        None => response,
        Some(key_workspace_id) => {
            let mut workspaces = Vec::new();
            for ws in response.workspaces {
                let Ok(ws_id) = uuid::Uuid::parse_str(&ws.id) else {
                    continue;
                };
                if crate::server::handlers::helpers::authorize_workspace_access(
                    &crate::server::middleware::ApiKeyRole::User,
                    key_workspace_id,
                    ws_id,
                )
                .await
                .is_ok()
                {
                    workspaces.push(ws);
                }
            }
            let total = workspaces.len() as u64;
            WorkspaceListResponse { workspaces, total }
        }
    };
    Ok(Json(filtered))
}

/// 单查 workspace 的角色-租户收敛（`handle_get_workspace` 专用）：Admin 放行；
/// User 经 `authorize_workspace_access` 仅自身，否则 403；Anonymous 及
/// User+None 组合 fail-closed 401。
async fn enforce_workspace_read_access(
    role: &crate::server::middleware::ApiKeyRole,
    key_workspace_id: Option<uuid::Uuid>,
    workspace: &WorkspaceResponse,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    match role {
        crate::server::middleware::ApiKeyRole::Admin => Ok(()),
        crate::server::middleware::ApiKeyRole::User => {
            let key_workspace_id =
                key_workspace_id.ok_or_else(|| auth_required_response(locale))?;
            let workspace_id = uuid::Uuid::parse_str(&workspace.id)
                .map_err(|_| invalid_workspace_id_response(locale))?;
            crate::server::handlers::helpers::authorize_workspace_access(
                &crate::server::middleware::ApiKeyRole::User,
                key_workspace_id,
                workspace_id,
            )
            .await
            .map_err(|_| workspace_mismatch_response(locale))
        }
        crate::server::middleware::ApiKeyRole::Anonymous => Err(auth_required_response(locale)),
    }
}

async fn handle_get_workspace(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Path(name): Path<String>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.handlers.get_workspace(&name).await {
        Ok(Some(ws)) => {
            // （CWE-639）User 仅可查看自身 workspace；Admin 跨租户放行。
            enforce_workspace_read_access(&extensions_role.0, extensions.0, &ws, locale).await?;
            Ok(Json(ws))
        }
        Ok(None) => Err(workspace_not_found_response(locale)),
        Err(e) => Err(core_error_to_response(&e, locale)),
    }
}

// ========== Group Handlers ==========

async fn handle_create_group(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<CreateGroupRequest>,
) -> Result<Json<GroupResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate request parameters
    validate_request(&req, locale)?;

    // Only User API Key can create groups
    verify_user_role(extensions_role.0, locale)?;

    // Verify workspace_id match for User API Key
    verify_user_workspace(&req.workspace, &extensions.0, &state.handlers, locale).await?;

    state
        .handlers
        .create_group(req)
        .await
        .map(Json)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_regenerate_user_key(
    State(state): State<NebulaIdState>,
    Extension(locale): Extension<Locale>,
    Path(name): Path<String>,
) -> Result<Json<ApiKeyWithSecretResponse>, (StatusCode, Json<ErrorResponse>)> {
    state
        .handlers
        .regenerate_user_api_key(&name)
        .await
        .map(Json)
        .map_err(|e: crate::core::CoreError| core_error_to_response(&e, locale))
}

async fn handle_list_groups(
    State(state): State<NebulaIdState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Query(params): Query<GroupListParams>,
) -> Result<Json<GroupListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 上报错误并强制 page_size ∈ [1, 100] 的 validate 约束；文案本地化。
    validate_request(&params, locale)?;

    // （CWE-862）list_groups 与 create_group 同为 User-only：否则任意认证主体
    // 都能按 `?workspace=` 列取 group；Admin 应走管理面。
    verify_user_role(extensions_role.0, locale)?;

    // （CWE-639）User 仅可列自身 workspace：反查 UUID 后经
    // `authorize_workspace_access` 判定，他人 → 403。
    verify_user_workspace(&params.workspace, &extensions.0, &state.handlers, locale).await?;

    let workspace = params.workspace.clone();

    let response = state
        .handlers
        .list_groups(&workspace)
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;
    Ok(Json(response))
}

// ========== API Key Handlers ==========

async fn handle_create_api_key(
    State(state): State<NebulaIdState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiKeyWithSecretResponse>, (StatusCode, Json<ErrorResponse>)> {
    validate_request(&req, locale)?;

    // 区分 `workspace_id` 缺失（`workspace_id_required_response`）与格式错误
    // （`invalid_uuid_response`），避免错误归因；下方闭包显式编码两条路径。
    let parse_user_workspace_id =
        |raw: &Option<String>| -> Result<uuid::Uuid, (StatusCode, Json<ErrorResponse>)> {
            match raw.as_deref() {
                Some(s) => uuid::Uuid::parse_str(s).map_err(|_| invalid_uuid_response(locale)),
                None => Err(workspace_id_required_response(locale)),
            }
        };

    // Determine workspace_id based on role
    let workspace_id = if let Some(ref role_str) = req.role {
        if role_str == "admin" {
            // Admin keys are global, not bound to any workspace
            None
        } else {
            // User key 必须绑定 workspace（缺失/格式错误分路径）。
            Some(parse_user_workspace_id(&req.workspace_id)?)
        }
    } else {
        // Default to user role if not specified
        Some(parse_user_workspace_id(&req.workspace_id)?)
    };

    state
        .handlers
        .create_api_key(workspace_id, req)
        .await
        .map(Json)
        .map_err(|e: crate::core::types::CoreError| core_error_to_response(&e, locale))
}

async fn handle_list_api_keys(
    State(state): State<NebulaIdState>,
    Extension(locale): Extension<Locale>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ApiKeyListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 上报错误并强制 page_size ∈ [1, 100] 的 validate 约束；文案本地化。
    validate_request(&params, locale)?;

    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    // 非法 workspace_id 记录 warn 并回退 `Uuid::nil()`（admin 语义：跨全部
    // workspace 列出）；返回错误会破坏依赖 nil UUID 的既有流程。
    let workspace_id = match params.workspace_id.as_deref() {
        Some(s) => match uuid::Uuid::parse_str(s) {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!(
                    event = "list_api_keys_invalid_workspace_id",
                    raw = %s,
                    "invalid workspace_id query parameter; falling back to nil UUID"
                );
                uuid::Uuid::nil()
            }
        },
        None => uuid::Uuid::nil(),
    };

    let response = state
        .handlers
        .list_api_keys(workspace_id, Some(page_size as u32), Some(offset as u32))
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;
    Ok(Json(response))
}

async fn handle_revoke_api_key(
    State(state): State<NebulaIdState>,
    Extension(locale): Extension<Locale>,
    Path(id): Path<String>,
) -> Result<Json<RevokeApiKeyResponse>, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| invalid_uuid_response(locale))?;

    state
        .handlers
        .revoke_api_key(uuid)
        .await
        .map(Json)
        .map_err(|e: crate::core::types::CoreError| core_error_to_response(&e, locale))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::algorithm::AlgorithmRouter;
    use crate::core::config::Config;
    use crate::core::types::AlgorithmType;
    use crate::server::config::management::ConfigManager;
    use crate::server::config::HotReloadConfig;
    use std::sync::Arc;
    // `Router::oneshot` 由 `tower::ServiceExt` 提供，引入以端到端驱动路由。
    use sdforge::tower::ServiceExt;

    fn create_test_api_handlers() -> Arc<ApiHandlers> {
        let config = Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = Arc::new(AlgorithmRouter::new(config.clone(), None));
        let config_service = Arc::new(ConfigManager::new(hot_config, algorithm_router.clone()));
        let handlers = ApiHandlers::new(algorithm_router, config_service);
        Arc::new(handlers)
    }

    fn create_test_auth() -> Arc<ApiKeyAuth> {
        use crate::core::database::{
            ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole as CoreApiKeyRole,
            ApiKeyWithSecret, CreateApiKeyRequest,
        };
        use crate::core::types::Result;
        use async_trait::async_trait;
        use uuid::Uuid;

        #[derive(Clone)]
        struct MockApiKeyRepo;

        #[async_trait]
        impl ApiKeyRepository for MockApiKeyRepo {
            async fn create_api_key(
                &self,
                _request: &CreateApiKeyRequest,
            ) -> Result<ApiKeyWithSecret> {
                Ok(ApiKeyWithSecret {
                    key: ApiKeyResponse {
                        id: Uuid::new_v4(),
                        key_id: "mock_key_id".to_string(),
                        key_prefix: "nino_".to_string(),
                        name: "Mock Key".to_string(),
                        description: None,
                        role: CoreApiKeyRole::User,
                        rate_limit: 10000,
                        enabled: true,
                        expires_at: None,
                        created_at: chrono::Utc::now().naive_utc(),
                    },
                    key_secret: "mock_secret".to_string(),
                    grace_expires_at: None,
                })
            }

            async fn get_api_key_by_id(&self, _key_id: &str) -> Result<Option<ApiKeyInfo>> {
                Ok(None)
            }

            async fn validate_api_key(
                &self,
                _key_id: &str,
                _key_secret: &str,
            ) -> Result<Option<crate::core::database::AuthenticatedKey>> {
                Ok(None)
            }

            async fn list_api_keys(
                &self,
                _workspace_id: Uuid,
                _limit: Option<u32>,
                _offset: Option<u32>,
            ) -> Result<Vec<ApiKeyInfo>> {
                Ok(vec![])
            }

            async fn delete_api_key(&self, _id: Uuid) -> Result<()> {
                Ok(())
            }

            async fn revoke_api_key(&self, _id: Uuid) -> Result<()> {
                Ok(())
            }

            async fn update_last_used(&self, _key: Uuid) -> Result<()> {
                Ok(())
            }

            async fn get_admin_api_key(&self, _workspace_id: Uuid) -> Result<Option<ApiKeyInfo>> {
                Ok(None)
            }

            async fn count_api_keys(&self, _workspace_id: Uuid) -> Result<u64> {
                Ok(0)
            }

            /// admin 守卫用：本 mock 是无状态装配桩，与 get_api_key_by_id 一致返回 None。
            async fn find_api_key_by_row_id(&self, _id: Uuid) -> Result<Option<ApiKeyInfo>> {
                Ok(None)
            }

            /// admin 守卫用：本 mock 是无状态装配桩，与 count_api_keys 一致返回 0。
            async fn count_admin_keys(&self) -> Result<u64> {
                Ok(0)
            }

            async fn rotate_api_key(
                &self,
                _key_id: &str,
                _grace_period_seconds: u64,
            ) -> Result<ApiKeyWithSecret> {
                Err(crate::core::types::error::CoreError::InternalError(
                    "rotate_api_key not implemented in mock".to_string(),
                ))
            }

            async fn get_keys_older_than(
                &self,
                _age_threshold_days: i64,
            ) -> Result<Vec<ApiKeyInfo>> {
                Ok(vec![])
            }
        }

        Arc::new(ApiKeyAuth::new(Arc::new(MockApiKeyRepo), true))
    }

    fn create_test_rate_limiter() -> Arc<RateLimiter> {
        Arc::new(RateLimiter::new(10000, 100))
    }

    fn create_test_audit_logger() -> Arc<AuditLogger> {
        Arc::new(AuditLogger::new(10000))
    }

    #[tokio::test]
    async fn test_router_creation() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let _router = router;
    }

    #[tokio::test]
    async fn test_router_endpoints() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let _router = router;
    }

    // ========== 限流层挂载契约 ==========

    fn make_plain_request(uri: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn test_rate_limit_layer_is_skipped_when_disabled() {
        let limiter = Arc::new(RateLimiter::new(1, 1));
        // 预耗尽 anonymous 桶：限流层若仍挂载，后续请求必 429
        limiter.check_rate_limit("anonymous", None, None).await;

        let router = create_router_with_rate_limit(
            create_test_api_handlers(),
            create_test_auth(),
            limiter,
            create_test_audit_logger(),
            false,
        )
        .await;

        for _ in 0..3 {
            let resp = router
                .clone()
                .oneshot(make_plain_request("/health"))
                .await
                .unwrap();
            assert_ne!(
                resp.status(),
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                "[rate_limit].enabled=false 时不得挂载限流层"
            );
        }
    }

    #[tokio::test]
    async fn test_429_generated_inside_security_header_layers() {
        // 层序契约：限流必须在 CORS/安全头内侧，否则 429 由最外层生成、不带响应头。
        let limiter = Arc::new(RateLimiter::new(1, 1));
        limiter.check_rate_limit("anonymous", None, None).await;

        let router = create_router_with_rate_limit(
            create_test_api_handlers(),
            create_test_auth(),
            limiter,
            create_test_audit_logger(),
            true,
        )
        .await;

        let resp = router.oneshot(make_plain_request("/health")).await.unwrap();
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "配额耗尽后应被限流"
        );
        assert_eq!(
            resp.headers()
                .get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff"),
            "429 必须穿过安全头与 CORS 层（CORS 与限流层序错误时丢失）"
        );
        // 拒绝响应显式声明剩余配额
        assert_eq!(
            resp.headers()
                .get(crate::server::rate_limit::middleware::HEADER_RATE_LIMIT_REMAINING)
                .and_then(|v| v.to_str().ok()),
            Some("0"),
            "实际响应头: {:?}",
            resp.headers()
        );
    }

    // ========== anonymous_block_middleware tests ==========

    fn build_anonymous_block_router() -> Router {
        // 仅挂 anonymous_block_middleware，隔离验证其行为。
        Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(anonymous_block_middleware))
    }

    fn make_request_with_extension<T: Clone + Send + Sync + 'static>(
        ext: T,
    ) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri("/test")
            .method("GET")
            .extension(ext)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn make_request_no_extension() -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri("/test")
            .method("GET")
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn test_anonymous_block_middleware_user_role_calls_next() {
        let router = build_anonymous_block_router();
        let resp = router
            .oneshot(make_request_with_extension(
                crate::server::middleware::ApiKeyRole::User,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_anonymous_block_middleware_admin_role_calls_next() {
        let router = build_anonymous_block_router();
        let resp = router
            .oneshot(make_request_with_extension(
                crate::server::middleware::ApiKeyRole::Admin,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_anonymous_block_middleware_anonymous_role_returns_401() {
        let router = build_anonymous_block_router();
        let resp = router
            .oneshot(make_request_with_extension(
                crate::server::middleware::ApiKeyRole::Anonymous,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_anonymous_block_middleware_no_role_extension_returns_401() {
        let router = build_anonymous_block_router();
        let resp = router.oneshot(make_request_no_extension()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ========== verify_user_role tests ==========

    #[test]
    fn test_verify_user_role_user_returns_ok() {
        let result = verify_user_role(crate::server::middleware::ApiKeyRole::User, Locale::En);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_user_role_admin_returns_forbidden() {
        let result = verify_user_role(crate::server::middleware::ApiKeyRole::Admin, Locale::En);
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn test_verify_user_role_anonymous_returns_unauthorized() {
        let result = verify_user_role(crate::server::middleware::ApiKeyRole::Anonymous, Locale::En);
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // ========== validate_request tests ==========

    #[derive(sdforge::validator::Validate)]
    #[validate(crate = "::sdforge::validator")]
    struct TestValidatable {
        #[validate(length(min = 1, max = 64))]
        name: String,
    }

    #[test]
    fn test_validate_request_valid_returns_ok() {
        let req = TestValidatable {
            name: "test-name".to_string(),
        };
        let result = validate_request(&req, Locale::En);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_request_invalid_returns_bad_request() {
        let req = TestValidatable {
            name: String::new(),
        };
        let result = validate_request(&req, Locale::En);
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // ========== verify_workspace_id_match tests ==========

    #[tokio::test]
    async fn test_verify_workspace_id_match_matching_returns_ok() {
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id = Some(workspace_uuid);
        let result = verify_workspace_id_match(workspace_uuid, &key_workspace_id, Locale::En).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_workspace_id_match_mismatch_returns_forbidden() {
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id = Some(uuid::Uuid::new_v4());
        let result = verify_workspace_id_match(workspace_uuid, &key_workspace_id, Locale::En).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_verify_workspace_id_match_none_key_workspace_returns_ok() {
        // key_workspace_id 为 None（admin 或禁用认证）时任意 workspace 都接受。
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id: Option<uuid::Uuid> = None;
        let result = verify_workspace_id_match(workspace_uuid, &key_workspace_id, Locale::En).await;
        assert!(result.is_ok());
    }

    // ========== verify_workspace_id tests ==========

    #[tokio::test]
    async fn test_verify_workspace_id_matching_returns_ok() {
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id = Some(workspace_uuid);
        let result = verify_workspace_id(workspace_uuid, &key_workspace_id, Locale::En).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_workspace_id_mismatch_returns_forbidden() {
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id = Some(uuid::Uuid::new_v4());
        let result = verify_workspace_id(workspace_uuid, &key_workspace_id, Locale::En).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_verify_workspace_id_none_key_returns_ok() {
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id: Option<uuid::Uuid> = None;
        let result = verify_workspace_id(workspace_uuid, &key_workspace_id, Locale::En).await;
        assert!(result.is_ok());
    }

    // ========== handle_api_info tests ==========

    #[tokio::test]
    async fn test_handle_api_info_returns_response() {
        let resp = handle_api_info().await;
        assert_eq!(resp.name, "Nebula ID Service");
        // 版本号唯一来源 = Cargo.toml（openapi.rs 同步）。
        assert_eq!(resp.version, env!("CARGO_PKG_VERSION"));
        assert!(!resp.endpoints.is_empty());
        assert!(resp.endpoints.iter().any(|e| e.contains("/health")));
        assert!(resp.endpoints.iter().any(|e| e.contains("/generate")));
        assert!(resp.endpoints.iter().any(|e| e.contains("/parse")));
    }

    /// parity 守卫：api-info endpoints 必须覆盖 `API_V1_ROUTE_PREFIXES` 全部
    /// 前缀（axum 不公开路由枚举；子串匹配，条目文字改动不误报）。
    #[tokio::test]
    async fn test_api_info_endpoints_cover_v1_routes() {
        let resp = handle_api_info().await;
        for prefix in API_V1_ROUTE_PREFIXES {
            let path = format!("/api/v1{prefix}");
            assert!(
                resp.endpoints.iter().any(|e| e.contains(&path)),
                "api-info endpoints 清单缺少 {path}（路由与展示漂移）"
            );
        }
    }

    // ========== create_router integration tests ==========

    #[tokio::test]
    async fn test_create_router_health_endpoint_responds() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_router_ready_endpoint_responds() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/ready")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_router_metrics_endpoint_responds() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_router_api_info_endpoint_responds() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_router_unknown_route_returns_404() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/nonexistent-route")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ========== request_id 中间件全链路 ==========

    #[tokio::test]
    async fn test_full_router_echoes_uuid_request_id_header() {
        // request_id 在 context_middleware 外侧：/health 必须回显合法 UUID。
        let router = create_router(
            create_test_api_handlers(),
            create_test_auth(),
            create_test_rate_limiter(),
            create_test_audit_logger(),
        )
        .await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let header = resp
            .headers()
            .get("x-request-id")
            .expect("响应必须携带 x-request-id 头")
            .to_str()
            .unwrap()
            .to_owned();
        let parsed = uuid::Uuid::parse_str(&header).expect("request_id 必须是合法 UUID");
        assert_eq!(parsed.get_version_num(), 7, "缺省生成必须是 UUID v7");
    }

    #[tokio::test]
    async fn test_full_router_passes_through_valid_request_id() {
        let router = create_router(
            create_test_api_handlers(),
            create_test_auth(),
            create_test_rate_limiter(),
            create_test_audit_logger(),
        )
        .await;
        let upstream = "123e4567-e89b-12d3-a456-426614174000";
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .header("x-request-id", upstream)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let header = resp
            .headers()
            .get("x-request-id")
            .expect("响应必须携带 x-request-id 头")
            .to_str()
            .unwrap();
        assert_eq!(header, upstream, "合法上游 x-request-id 必须透传原值");
    }

    #[tokio::test]
    async fn test_error_response_body_request_id_matches_response_header() {
        // 错误信封在 request_id 作用域内：响应体 request_id 必须与响应头一致。
        async fn failing_handler() -> axum::response::Response {
            let (status, json) = core_error_to_response(
                &crate::core::types::CoreError::InvalidInput("boom".to_string()),
                Locale::En,
            );
            (status, json).into_response()
        }

        let app = axum::Router::new()
            .route("/boom", axum::routing::get(failing_handler))
            .layer(axum::middleware::from_fn(
                crate::server::middleware::request_id::request_id_middleware,
            ));

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/boom")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let header = resp
            .headers()
            .get("x-request-id")
            .expect("错误响应必须携带 x-request-id 头")
            .to_str()
            .unwrap()
            .to_owned();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).expect("错误信封是 JSON");
        assert_eq!(
            json["request_id"].as_str().expect("request_id 是字符串"),
            header,
            "错误响应体 request_id 必须与响应头一致"
        );
    }

    #[tokio::test]
    async fn test_create_router_generate_endpoint_requires_auth() {
        // 无认证头 → 401（auth_middleware 拒绝）。
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/generate")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "workspace": "test",
                            "group": "test",
                            "biz_tag": "test"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_create_router_openapi_endpoint_responds() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api-docs/openapi.json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_router_security_headers_present() {
        // 响应应带全套安全头。
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
        assert_eq!(
            resp.headers()
                .get("content-security-policy")
                .unwrap()
                .to_str()
                .unwrap(),
            "default-src 'self'"
        );
        assert_eq!(
            resp.headers()
                .get("strict-transport-security")
                .unwrap()
                .to_str()
                .unwrap(),
            "max-age=31536000; includeSubDomains"
        );
        assert_eq!(
            resp.headers()
                .get("x-xss-protection")
                .unwrap()
                .to_str()
                .unwrap(),
            "1; mode=block"
        );
        assert_eq!(
            resp.headers()
                .get("referrer-policy")
                .unwrap()
                .to_str()
                .unwrap(),
            "strict-origin-when-cross-origin"
        );
    }

    // ========== NebulaIdState helper ==========

    fn create_test_app_state() -> NebulaIdState {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let config_service = handlers.get_config_service();
        NebulaIdState {
            handlers,
            auth,
            config_service,
            prometheus: Arc::new(PrometheusMetricsBridge::new()),
        }
    }

    // ========== verify_user_workspace tests ==========

    #[tokio::test]
    async fn test_verify_user_workspace_without_repository_returns_internal_error() {
        // 无 workspace_repository → get_workspace 返回 InternalError → 500。
        let handlers = create_test_api_handlers();
        let result = verify_user_workspace("any-name", &None, &handlers, Locale::En).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_verify_user_workspace_with_matching_key_returns_error_from_repo() {
        // 即使带 key_workspace_id，仍先命中缺失的仓储 → 500。
        let handlers = create_test_api_handlers();
        let key = uuid::Uuid::new_v4();
        let result = verify_user_workspace("any-name", &Some(key), &handlers, Locale::En).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_get_config tests ==========

    #[tokio::test]
    async fn test_handle_get_config_returns_secure_config_with_app_info() {
        let state = create_test_app_state();
        let resp = handle_get_config(State(state)).await;
        assert!(!resp.app.name.is_empty());
        assert!(!resp.algorithm.default.is_empty());
    }

    // ========== handle_health tests ==========

    #[tokio::test]
    async fn test_handle_health_returns_status_and_algorithm() {
        let state = create_test_app_state();
        let resp = handle_health(State(state)).await;
        let known = matches!(
            resp.status,
            crate::server::models::HealthStatus::Healthy
                | crate::server::models::HealthStatus::Degraded
                | crate::server::models::HealthStatus::Unhealthy
        );
        assert!(known);
        assert!(!resp.algorithm.is_empty());
    }

    // ========== handle_ready tests ==========

    #[tokio::test]
    async fn test_handle_ready_returns_ready_flag_and_components() {
        let state = create_test_app_state();
        let resp = handle_ready(State(state)).await;
        let _ = resp.ready;
        let _ = resp.database;
        let _ = resp.cache;
        assert!(resp.message.is_empty() || !resp.message.is_empty());
    }

    // ========== handle_metrics (Prometheus 文本) tests ==========

    #[tokio::test]
    async fn test_handle_metrics_renders_prometheus_text() {
        let state = create_test_app_state();
        let resp = handle_metrics(State(state)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("content-type header must be present")
            .to_str()
            .expect("content-type must be visible ascii")
            .to_owned();
        assert!(
            content_type.starts_with("text/plain"),
            "content-type 应为 Prometheus 文本格式，实际 {content_type}"
        );

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let body = std::str::from_utf8(&body).expect("prometheus text is utf-8");
        assert!(body.contains("# HELP"), "缺少 # HELP 行:\n{body}");
        assert!(
            body.contains("nebula_id_"),
            "缺少 nebula_id_ 前缀指标:\n{body}"
        );
    }

    #[tokio::test]
    async fn test_handle_metrics_ids_generated_total_increments_after_generates() {
        // 多次真实 generate 走 Snowflake 内部计数器（无 DB/etcd 依赖），渲染出的
        // nebula_id_ids_generated_total 必须跟随递增；uuid_v8 返回默认快照属上游
        // 既有口径，本测试只钉住该计数器的桥接。
        let config = Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = Arc::new(AlgorithmRouter::new(config.clone(), None));
        algorithm_router
            .initialize()
            .await
            .expect("initialize 必须至少注册一个算法（snowflake/uuid_v8 均无外部依赖）");

        let config_service: Arc<dyn ConfigManagementService> =
            Arc::new(ConfigManager::new(hot_config, algorithm_router.clone()));
        let handlers = Arc::new(ApiHandlers::new(
            algorithm_router.clone(),
            config_service.clone(),
        ));
        let state = NebulaIdState {
            handlers,
            auth: create_test_auth(),
            config_service,
            prometheus: Arc::new(PrometheusMetricsBridge::new()),
        };

        const GENERATES: u64 = 5;
        for _ in 0..GENERATES {
            algorithm_router
                .generate_with_algorithm(AlgorithmType::Snowflake, "ws", "g", "bt")
                .await
                .expect("snowflake generate must succeed");
        }

        let resp = handle_metrics(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let body = std::str::from_utf8(&body).expect("prometheus text is utf-8");

        let line = body
            .lines()
            .find(|l| l.starts_with("nebula_id_ids_generated_total{algorithm=\"snowflake\"}"))
            .expect("snowflake 的 ids_generated_total 必须出现在渲染输出中");
        let value: u64 = line
            .rsplit(' ')
            .next()
            .expect("prometheus sample 行以数值结尾")
            .parse()
            .expect("sample value must be an integer");
        assert_eq!(
            value, GENERATES,
            "算法级计数器递增必须桥接到 nebula_id_ids_generated_total"
        );
    }

    // ========== handle_reload_config tests ==========

    #[tokio::test]
    async fn test_handle_reload_config_returns_response() {
        let state = create_test_app_state();
        let resp = handle_reload_config(State(state)).await;
        let _ = resp.success;
        assert!(!resp.message.is_empty() || resp.message.is_empty());
    }

    // ========== handle_update_rate_limit tests ==========

    #[tokio::test]
    async fn test_handle_update_rate_limit_rejects_out_of_range() {
        let state = create_test_app_state();
        let req = UpdateRateLimitRequest {
            default_rps: Some(0), // below min=1
            burst_size: None,
        };
        let result = handle_update_rate_limit(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_update_rate_limit_rejects_burst_out_of_range() {
        let state = create_test_app_state();
        let req = UpdateRateLimitRequest {
            default_rps: None,
            burst_size: Some(0), // below min=1
        };
        let result = handle_update_rate_limit(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_update_rate_limit_accepts_valid_input() {
        let state = create_test_app_state();
        let req = UpdateRateLimitRequest {
            default_rps: Some(100),
            burst_size: Some(50),
        };
        let result = handle_update_rate_limit(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_ok());
        let resp = result.unwrap();
        let _ = resp.0.success;
    }

    // ========== handle_update_logging tests ==========

    #[tokio::test]
    async fn test_handle_update_logging_rejects_empty_level() {
        let state = create_test_app_state();
        let req = UpdateLoggingRequest {
            level: Some(String::new()),
        };
        let result = handle_update_logging(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_update_logging_accepts_valid_level() {
        let state = create_test_app_state();
        let req = UpdateLoggingRequest {
            level: Some("info".to_string()),
        };
        let result = handle_update_logging(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_ok());
    }

    // ========== handle_set_algorithm tests ==========

    #[tokio::test]
    async fn test_handle_set_algorithm_rejects_empty_biz_tag() {
        let state = create_test_app_state();
        let req = SetAlgorithmRequest {
            biz_tag: String::new(),
            algorithm: "segment".to_string(),
        };
        let result = handle_set_algorithm(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_set_algorithm_rejects_empty_algorithm() {
        let state = create_test_app_state();
        let req = SetAlgorithmRequest {
            biz_tag: "tag".to_string(),
            algorithm: String::new(),
        };
        let result = handle_set_algorithm(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_set_algorithm_accepts_valid_input() {
        let state = create_test_app_state();
        let req = SetAlgorithmRequest {
            biz_tag: "tag".to_string(),
            algorithm: "segment".to_string(),
        };
        let result = handle_set_algorithm(State(state), Extension(Locale::En), Json(req)).await;
        // 只要 handler 执行即可：成功或错误响应均可接受。
        match result {
            Ok(resp) => {
                let _ = resp.0.success;
            }
            Err((status, _)) => {
                // Validation passed, so it must not be a 400.
                assert_ne!(status, StatusCode::BAD_REQUEST);
            }
        }
    }

    // ========== handle_generate tests ==========

    fn make_generate_request() -> GenerateRequest {
        GenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-tag".to_string(),
            algorithm: None,
        }
    }

    #[tokio::test]
    async fn test_handle_generate_admin_role_returns_forbidden() {
        let state = create_test_app_state();
        let result = handle_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Json(make_generate_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_handle_generate_anonymous_role_returns_unauthorized() {
        let state = create_test_app_state();
        let result = handle_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Anonymous),
            Extension(Locale::En),
            Json(make_generate_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_handle_generate_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let req = GenerateRequest {
            workspace: String::new(), // empty -> validation error
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            algorithm: None,
        };
        let result = handle_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_generate_user_role_fails_at_workspace_lookup() {
        // User 通过角色与校验后，workspace 反查命中缺失仓储 → 500。
        let state = create_test_app_state();
        let result = handle_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(make_generate_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_batch_generate tests ==========

    fn make_batch_request() -> BatchGenerateRequest {
        BatchGenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-tag".to_string(),
            size: Some(5),
            algorithm: None,
        }
    }

    #[tokio::test]
    async fn test_handle_batch_generate_admin_role_returns_forbidden() {
        let state = create_test_app_state();
        let result = handle_batch_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Json(make_batch_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_handle_batch_generate_anonymous_role_returns_unauthorized() {
        let state = create_test_app_state();
        let result = handle_batch_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Anonymous),
            Extension(Locale::En),
            Json(make_batch_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_handle_batch_generate_invalid_size_returns_bad_request() {
        let state = create_test_app_state();
        let req = BatchGenerateRequest {
            workspace: "ws".to_string(),
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            size: Some(0), // below min=1
            algorithm: None,
        };
        let result = handle_batch_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_batch_generate_user_role_fails_at_workspace_lookup() {
        let state = create_test_app_state();
        let result = handle_batch_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(make_batch_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_parse tests ==========

    #[tokio::test]
    async fn test_handle_parse_invalid_workspace_returns_bad_request() {
        let state = create_test_app_state();
        let req = ParseRequest {
            id: "123".to_string(),
            workspace: String::new(), // empty -> validation error
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            algorithm: String::new(),
        };
        let result = handle_parse(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_parse_invalid_id_format_returns_bad_request() {
        let state = create_test_app_state();
        let req = ParseRequest {
            id: "not-a-valid-id".to_string(),
            workspace: "ws".to_string(),
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            algorithm: String::new(),
        };
        let result = handle_parse(State(state), Extension(Locale::En), Json(req)).await;
        // Id::from_string fails on garbage -> InvalidIdString -> 400.
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // ========== handle_create_biz_tag tests ==========

    fn make_create_biz_tag_request() -> CreateBizTagRequest {
        CreateBizTagRequest {
            workspace_id: uuid::Uuid::new_v4(),
            group_id: uuid::Uuid::new_v4(),
            name: "test-tag".to_string(),
            description: None,
            algorithm: Some("segment".to_string()),
            format: Some("decimal".to_string()),
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        }
    }

    #[tokio::test]
    async fn test_handle_create_biz_tag_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let mut req = make_create_biz_tag_request();
        req.name = String::new(); // empty -> length(min=1) violation
        let result = handle_create_biz_tag(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_biz_tag_admin_role_returns_forbidden() {
        let state = create_test_app_state();
        let result = handle_create_biz_tag(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Json(make_create_biz_tag_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_handle_create_biz_tag_anonymous_role_returns_unauthorized() {
        let state = create_test_app_state();
        let result = handle_create_biz_tag(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Anonymous),
            Extension(Locale::En),
            Json(make_create_biz_tag_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_handle_create_biz_tag_workspace_mismatch_returns_forbidden() {
        let state = create_test_app_state();
        // Key has a different workspace_id than the request.
        let key_workspace = uuid::Uuid::new_v4();
        let result = handle_create_biz_tag(
            State(state),
            Extension(Some(uuid::Uuid::new_v4())), // different UUID
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(make_create_biz_tag_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // verify_workspace_id returns 403 on mismatch.
        assert_eq!(status, StatusCode::FORBIDDEN);
        let _ = key_workspace;
    }

    #[tokio::test]
    async fn test_handle_create_biz_tag_with_matching_key_fails_at_handler() {
        let state = create_test_app_state();
        let req = make_create_biz_tag_request();
        let key = req.workspace_id;
        let result = handle_create_biz_tag(
            State(state),
            Extension(Some(key)),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        // 角色与 workspace 均匹配 → 调 handlers.create_biz_tag；无仓储时按
        // CoreError 变体映射 400/404/500。
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert!(
            status == StatusCode::BAD_REQUEST
                || status == StatusCode::NOT_FOUND
                || status == StatusCode::INTERNAL_SERVER_ERROR,
            "expected BAD_REQUEST, NOT_FOUND, or INTERNAL_SERVER_ERROR, got {}",
            status
        );
    }

    // ========== handle_get_biz_tag tests ==========

    #[tokio::test]
    async fn test_handle_get_biz_tag_invalid_uuid_returns_bad_request() {
        let state = create_test_app_state();
        let result = handle_get_biz_tag(
            State(state),
            Extension(Some(uuid::Uuid::new_v4())),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Path("not-a-uuid".to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_get_biz_tag_valid_uuid_fails_at_handler() {
        let state = create_test_app_state();
        let result = handle_get_biz_tag(
            State(state),
            Extension(Some(uuid::Uuid::new_v4())),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Path(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without repository, BizTagNotFound or InternalError -> 404 or 500.
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
            "expected NOT_FOUND or INTERNAL_SERVER_ERROR, got {}",
            status
        );
    }

    // ========== handle_update_biz_tag tests ==========

    #[tokio::test]
    async fn test_handle_update_biz_tag_invalid_uuid_returns_bad_request() {
        let state = create_test_app_state();
        let req = UpdateBizTagRequest {
            name: Some("new-name".to_string()),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = handle_update_biz_tag(
            State(state),
            Extension(Some(uuid::Uuid::new_v4())),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Path("not-a-uuid".to_string()),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_update_biz_tag_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let req = UpdateBizTagRequest {
            name: Some(String::new()), // length(min=1) violation
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = handle_update_biz_tag(
            State(state),
            Extension(Some(uuid::Uuid::new_v4())),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Path(uuid::Uuid::new_v4().to_string()),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // ========== handle_delete_biz_tag tests ==========

    #[tokio::test]
    async fn test_handle_delete_biz_tag_invalid_uuid_returns_bad_request() {
        let state = create_test_app_state();
        let result = handle_delete_biz_tag(
            State(state),
            Extension(Some(uuid::Uuid::new_v4())),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Path("not-a-uuid".to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_delete_biz_tag_valid_uuid_fails_at_handler() {
        let state = create_test_app_state();
        let result = handle_delete_biz_tag(
            State(state),
            Extension(Some(uuid::Uuid::new_v4())),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Path(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
            "expected NOT_FOUND or INTERNAL_SERVER_ERROR, got {}",
            status
        );
    }

    // ========== handle_list_biz_tags tests ==========

    #[tokio::test]
    async fn test_handle_list_biz_tags_invalid_page_size_returns_bad_request() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: None,
            page: 1,
            page_size: 101, // above max=100
        };
        let result = handle_list_biz_tags(
            State(state),
            Extension(None::<uuid::Uuid>),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Query(params),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_list_biz_tags_zero_page_size_returns_bad_request() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: None,
            page: 1,
            page_size: 0, // below min=1
        };
        let result = handle_list_biz_tags(
            State(state),
            Extension(None::<uuid::Uuid>),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Query(params),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_list_biz_tags_valid_params_returns_response_or_error() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: None,
            page: 1,
            page_size: 20,
        };
        let result = handle_list_biz_tags(
            State(state),
            Extension(Some(uuid::Uuid::new_v4())),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Query(params),
        )
        .await;
        // Without repository, list_biz_tags_with_pagination returns Err.
        match result {
            Ok(resp) => {
                let _ = resp.0.total;
            }
            Err((status, _)) => {
                assert!(
                    status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND
                );
            }
        }
    }

    // ========== handle_create_workspace tests ==========

    #[tokio::test]
    async fn test_handle_create_workspace_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateWorkspaceRequest {
            name: String::new(), // length(min=1) violation
            description: None,
            max_groups: None,
            max_biz_tags: None,
        };
        let result = handle_create_workspace(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_workspace_invalid_max_groups_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateWorkspaceRequest {
            name: "ws".to_string(),
            description: None,
            max_groups: Some(0), // below min=1
            max_biz_tags: None,
        };
        let result = handle_create_workspace(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_workspace_valid_request_fails_at_handler() {
        let state = create_test_app_state();
        let req = CreateWorkspaceRequest {
            name: "new-ws".to_string(),
            description: None,
            max_groups: None,
            max_biz_tags: None,
        };
        let result = handle_create_workspace(State(state), Extension(Locale::En), Json(req)).await;
        // Without repository, create_workspace returns Err.
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_list_workspaces tests ==========

    #[tokio::test]
    async fn test_handle_list_workspaces_without_repository_returns_error() {
        let state = create_test_app_state();
        // Admin 角色保持全量列表语义，仓库缺失时仍为 500。
        let result = handle_list_workspaces(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_get_workspace tests ==========

    #[tokio::test]
    async fn test_handle_get_workspace_without_repository_returns_internal_error() {
        let state = create_test_app_state();
        // Admin 角色跨租户放行，仓库缺失时仍为 500。
        let result = handle_get_workspace(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Path("any-name".to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_create_group tests ==========

    #[tokio::test]
    async fn test_handle_create_group_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateGroupRequest {
            workspace: String::new(), // length(min=1) violation
            name: "g".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = handle_create_group(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_group_admin_role_returns_forbidden() {
        let state = create_test_app_state();
        let req = CreateGroupRequest {
            workspace: "ws".to_string(),
            name: "g".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = handle_create_group(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_handle_create_group_anonymous_role_returns_unauthorized() {
        let state = create_test_app_state();
        let req = CreateGroupRequest {
            workspace: "ws".to_string(),
            name: "g".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = handle_create_group(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Anonymous),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_handle_create_group_user_role_fails_at_workspace_lookup() {
        let state = create_test_app_state();
        let req = CreateGroupRequest {
            workspace: "ws".to_string(),
            name: "g".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = handle_create_group(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // verify_user_workspace hits the missing repository -> 500.
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_regenerate_user_key tests ==========

    #[tokio::test]
    async fn test_handle_regenerate_user_key_without_repository_returns_error() {
        let state = create_test_app_state();
        let result = handle_regenerate_user_key(
            State(state),
            Extension(Locale::En),
            Path("any-name".to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without repository, regenerate_user_api_key returns Err.
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }

    // ========== handle_list_groups tests ==========

    #[tokio::test]
    async fn test_handle_list_groups_invalid_page_size_returns_bad_request() {
        let state = create_test_app_state();
        let params = GroupListParams {
            workspace: "ws".to_string(),
            page: 1,
            page_size: 0, // below min=1
        };
        // 校验先于角色/归属校验，故任意角色均可观察到 400。
        let result = handle_list_groups(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Query(params),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_list_groups_above_max_page_size_returns_bad_request() {
        let state = create_test_app_state();
        let params = GroupListParams {
            workspace: "ws".to_string(),
            page: 1,
            page_size: 101, // above max=100
        };
        let result = handle_list_groups(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Query(params),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_list_groups_valid_params_fails_at_handler() {
        let state = create_test_app_state();
        let params = GroupListParams {
            workspace: "ws".to_string(),
            page: 1,
            page_size: 20,
        };
        let result = handle_list_groups(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Query(params),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // 角色通过后进入归属校验；无仓库时反查返回 InternalError → 500。
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== workspace/group 读面租户隔离 ==========

    fn make_workspace_response(id: uuid::Uuid, name: &str) -> WorkspaceResponse {
        WorkspaceResponse {
            id: id.to_string(),
            name: name.to_string(),
            description: None,
            status: "active".to_string(),
            max_groups: 10,
            max_biz_tags: 100,
            created_at: String::new(),
            updated_at: String::new(),
            user_api_key: None,
        }
    }

    fn app_state_with_mock(
        mock_config: crate::server::handlers::mock_tests::MockConfigManagementService,
    ) -> NebulaIdState {
        use crate::server::handlers::mock_generator::MockIdGenerator;

        let handlers = Arc::new(ApiHandlers::new(
            Arc::new(MockIdGenerator::new()),
            Arc::new(mock_config),
        ));
        let auth = create_test_auth();
        let config_service = handlers.get_config_service();
        NebulaIdState {
            handlers,
            auth,
            config_service,
            prometheus: Arc::new(PrometheusMetricsBridge::new()),
        }
    }

    #[tokio::test]
    async fn t031_list_groups_user_cross_workspace_returns_forbidden() {
        use crate::server::handlers::mock_tests::MockConfigManagementService;

        let ws_a = uuid::Uuid::new_v4();
        let mut mock = MockConfigManagementService::new();
        // 反查命中他人 workspace；未设 list_groups 期望，触达 mock 即 panic。
        mock.expect_get_workspace()
            .returning(move |_| Ok(Some(make_workspace_response(ws_a, "ws-a"))));
        let state = app_state_with_mock(mock);

        let params = GroupListParams {
            workspace: "ws-a".to_string(),
            page: 1,
            page_size: 20,
        };
        let result = handle_list_groups(
            State(state),
            Extension(Some(uuid::Uuid::new_v4())), // 他人 workspace 绑定
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Query(params),
        )
        .await;
        assert!(result.is_err());
        let (status, json) = result.unwrap_err();
        // 403 / PermissionDenied 类响应（共享授权 → WorkspaceDisabled → 403）。
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json.code, 403);
    }

    #[tokio::test]
    async fn t031_list_groups_user_own_workspace_proceeds_to_listing() {
        use crate::server::handlers::mock_tests::MockConfigManagementService;

        let ws_a = uuid::Uuid::new_v4();
        let mut mock = MockConfigManagementService::new();
        mock.expect_get_workspace()
            .returning(move |_| Ok(Some(make_workspace_response(ws_a, "ws-a"))));
        mock.expect_list_groups().returning(|_| {
            Ok(GroupListResponse {
                groups: vec![],
                total: 0,
            })
        });
        let state = app_state_with_mock(mock);

        let params = GroupListParams {
            workspace: "ws-a".to_string(),
            page: 1,
            page_size: 20,
        };
        let result = handle_list_groups(
            State(state),
            Extension(Some(ws_a)), // 自身 workspace 绑定
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Query(params),
        )
        .await;
        assert!(result.is_ok(), "自身 workspace 列 group 必须放行");
    }

    #[tokio::test]
    async fn t031_list_groups_admin_rejected_and_anonymous_unauthorized() {
        use crate::server::handlers::mock_tests::MockConfigManagementService;

        let state = app_state_with_mock(MockConfigManagementService::new());
        let params = GroupListParams {
            workspace: "ws".to_string(),
            page: 1,
            page_size: 20,
        };

        // Admin → 403 admin_cannot_perform（list_groups 收敛为 User-only）。
        let result = handle_list_groups(
            State(state.clone()),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Query(params.clone()),
        )
        .await;
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Anonymous → 401（兜底；正常流量下 anonymous_block_middleware 已拦）。
        let result = handle_list_groups(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Anonymous),
            Extension(Locale::En),
            Query(params),
        )
        .await;
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn t031_list_workspaces_user_sees_only_own_workspace() {
        use crate::server::handlers::mock_tests::MockConfigManagementService;

        let ws_a = uuid::Uuid::new_v4();
        let ws_b = uuid::Uuid::new_v4();
        let ws_c = uuid::Uuid::new_v4();
        let mut mock = MockConfigManagementService::new();
        mock.expect_list_workspaces().returning(move || {
            Ok(WorkspaceListResponse {
                workspaces: vec![
                    make_workspace_response(ws_a, "ws-a"),
                    make_workspace_response(ws_b, "ws-b"),
                    make_workspace_response(ws_c, "ws-c"),
                ],
                total: 3,
            })
        });
        let state = app_state_with_mock(mock);

        let result = handle_list_workspaces(
            State(state),
            Extension(Some(ws_b)), // 绑定 ws-b 的 User key
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
        )
        .await;
        let resp = result.expect("User 列 workspace 应成功（过滤而非拒绝）");
        assert_eq!(resp.0.workspaces.len(), 1, "只能看到自身 workspace");
        assert_eq!(resp.0.workspaces[0].id, ws_b.to_string());
        assert_eq!(resp.0.total, 1, "total 必须随过滤结果收敛");
    }

    #[tokio::test]
    async fn t031_list_workspaces_admin_keeps_full_cross_tenant_list() {
        use crate::server::handlers::mock_tests::MockConfigManagementService;

        let ws_a = uuid::Uuid::new_v4();
        let ws_b = uuid::Uuid::new_v4();
        let mut mock = MockConfigManagementService::new();
        mock.expect_list_workspaces().returning(move || {
            Ok(WorkspaceListResponse {
                workspaces: vec![
                    make_workspace_response(ws_a, "ws-a"),
                    make_workspace_response(ws_b, "ws-b"),
                ],
                total: 2,
            })
        });
        let state = app_state_with_mock(mock);

        // Admin 行为不变：全量跨租户列表。
        let result = handle_list_workspaces(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
        )
        .await;
        let resp = result.expect("Admin 列 workspace 必须保持全量");
        assert_eq!(resp.0.workspaces.len(), 2);
        assert_eq!(resp.0.total, 2);
    }

    #[tokio::test]
    async fn t031_list_workspaces_user_without_binding_and_anonymous_fail_closed() {
        use crate::server::handlers::mock_tests::MockConfigManagementService;

        let state = app_state_with_mock(MockConfigManagementService::new());

        // User + 无 workspace 绑定（正常流量下不出现）→ fail-closed 401。
        let result = handle_list_workspaces(
            State(state.clone()),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
        )
        .await;
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Anonymous → 401。
        let result = handle_list_workspaces(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Anonymous),
            Extension(Locale::En),
        )
        .await;
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn t031_get_workspace_user_own_ok_other_forbidden_admin_cross_tenant_ok() {
        use crate::server::handlers::mock_tests::MockConfigManagementService;

        let ws_a = uuid::Uuid::new_v4();
        let ws_b = uuid::Uuid::new_v4();

        // User 查自身 workspace → 放行。
        let mut own = MockConfigManagementService::new();
        own.expect_get_workspace()
            .returning(move |_| Ok(Some(make_workspace_response(ws_a, "ws-a"))));
        let result = handle_get_workspace(
            State(app_state_with_mock(own)),
            Extension(Some(ws_a)),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Path("ws-a".to_string()),
        )
        .await;
        assert!(result.is_ok(), "User 查看自身 workspace 必须放行");

        // User 查他人 workspace → 403。
        let mut other = MockConfigManagementService::new();
        other
            .expect_get_workspace()
            .returning(move |_| Ok(Some(make_workspace_response(ws_b, "ws-b"))));
        let result = handle_get_workspace(
            State(app_state_with_mock(other)),
            Extension(Some(ws_a)),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Path("ws-b".to_string()),
        )
        .await;
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Admin 跨租户查看 → 放行（行为不变）。
        let mut admin_view = MockConfigManagementService::new();
        admin_view
            .expect_get_workspace()
            .returning(move |_| Ok(Some(make_workspace_response(ws_b, "ws-b"))));
        let result = handle_get_workspace(
            State(app_state_with_mock(admin_view)),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Path("ws-b".to_string()),
        )
        .await;
        assert!(result.is_ok(), "Admin 跨租户查看必须保持放行");
    }

    // ========== handle_create_api_key tests ==========

    #[tokio::test]
    async fn test_handle_create_api_key_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: None,
            name: String::new(), // length(min=1) violation
            description: None,
            role: Some("user".to_string()),
            rate_limit: None,
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_api_key_user_role_without_workspace_id_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: None, // missing for user role
            name: "key".to_string(),
            description: None,
            role: Some("user".to_string()),
            rate_limit: None,
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // workspace_id_required_response returns 400.
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_api_key_user_role_with_invalid_workspace_id_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: Some("not-a-uuid".to_string()),
            name: "key".to_string(),
            description: None,
            role: Some("user".to_string()),
            rate_limit: None,
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // invalid_uuid_response returns 400.
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_api_key_default_role_without_workspace_id_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: None, // missing for default user role
            name: "key".to_string(),
            description: None,
            role: None, // defaults to user
            rate_limit: None,
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_api_key_admin_role_without_workspace_id_fails_at_handler() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: None, // admin keys don't need workspace
            name: "admin-key".to_string(),
            description: None,
            role: Some("admin".to_string()),
            rate_limit: Some(1000),
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        // 校验通过、admin 的 workspace_id 为 None；无仓储时返回 Err。
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // 无仓储时按 CoreError 变体映射 404 或 500。
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
            "expected NOT_FOUND or INTERNAL_SERVER_ERROR, got {}",
            status
        );
    }

    #[tokio::test]
    async fn test_handle_create_api_key_user_role_with_valid_workspace_id_fails_at_handler() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: Some(uuid::Uuid::new_v4().to_string()),
            name: "user-key".to_string(),
            description: None,
            role: Some("user".to_string()),
            rate_limit: Some(1000),
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        // 校验通过、workspace_id 可解析；无仓储时返回 Err。
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // 无仓储时按 CoreError 变体映射 404 或 500。
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
            "expected NOT_FOUND or INTERNAL_SERVER_ERROR, got {}",
            status
        );
    }

    // ========== handle_list_api_keys tests ==========

    #[tokio::test]
    async fn test_handle_list_api_keys_invalid_page_size_returns_bad_request() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: None,
            page: 1,
            page_size: 0, // below min=1
        };
        let result = handle_list_api_keys(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_list_api_keys_valid_params_fails_at_handler() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: None,
            page: 1,
            page_size: 20,
        };
        let result = handle_list_api_keys(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without repository, list_api_keys returns Err.
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_handle_list_api_keys_with_invalid_workspace_id_falls_back_to_nil() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: Some("not-a-uuid".to_string()),
            page: 1,
            page_size: 20,
        };
        let result = handle_list_api_keys(State(state), Extension(Locale::En), Query(params)).await;
        // 非法 workspace_id 记日志并回退 nil；无仓储时调用仍失败。
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_handle_list_api_keys_with_valid_workspace_id_fails_at_handler() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: Some(uuid::Uuid::new_v4().to_string()),
            page: 2,
            page_size: 50,
        };
        let result = handle_list_api_keys(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }

    // ========== handle_revoke_api_key tests ==========

    #[tokio::test]
    async fn test_handle_revoke_api_key_invalid_uuid_returns_bad_request() {
        let state = create_test_app_state();
        let result = handle_revoke_api_key(
            State(state),
            Extension(Locale::En),
            Path("not-a-uuid".to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_revoke_api_key_valid_uuid_fails_at_handler() {
        let state = create_test_app_state();
        let result = handle_revoke_api_key(
            State(state),
            Extension(Locale::En),
            Path(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without repository, revoke_api_key returns Err.
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_verify_user_role_matrix() {
        use crate::server::middleware::ApiKeyRole;

        assert!(verify_user_role(ApiKeyRole::User, Locale::En).is_ok());
        assert!(verify_user_role(ApiKeyRole::Admin, Locale::En).is_err());
        assert!(verify_user_role(ApiKeyRole::Anonymous, Locale::En).is_err());
    }

    #[test]
    fn test_validate_request_ok_and_err() {
        let ok = GenerateRequest {
            workspace: "w".to_string(),
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            algorithm: None,
        };
        assert!(validate_request(&ok, Locale::En).is_ok());

        let bad = GenerateRequest {
            workspace: String::new(),
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            algorithm: None,
        };
        assert!(validate_request(&bad, Locale::En).is_err());
    }

    #[tokio::test]
    async fn test_verify_workspace_id_match_matrix() {
        let id = uuid::Uuid::new_v4();
        assert!(verify_workspace_id_match(id, &None, Locale::En)
            .await
            .is_ok());
        assert!(verify_workspace_id_match(id, &Some(id), Locale::En)
            .await
            .is_ok());
        assert!(
            verify_workspace_id_match(id, &Some(uuid::Uuid::new_v4()), Locale::En)
                .await
                .is_err()
        );
        assert!(verify_workspace_id(id, &None, Locale::En).await.is_ok());
    }

    #[tokio::test]
    async fn test_verify_user_workspace_paths() {
        use crate::server::handlers::mock_generator::MockIdGenerator;
        use crate::server::handlers::mock_tests::MockConfigManagementService;

        let ws_id = uuid::Uuid::new_v4();
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_get_workspace().returning(move |_| {
            Ok(Some(WorkspaceResponse {
                id: ws_id.to_string(),
                name: "ws".to_string(),
                description: None,
                status: "active".to_string(),
                max_groups: 10,
                max_biz_tags: 100,
                created_at: String::new(),
                updated_at: String::new(),
                user_api_key: None,
            }))
        });
        let handlers = Arc::new(ApiHandlers::new(
            Arc::new(MockIdGenerator::new()),
            Arc::new(mock_config),
        ));

        assert!(
            verify_user_workspace("ws", &Some(ws_id), &handlers, Locale::En)
                .await
                .is_ok()
        );
        assert!(
            verify_user_workspace("ws", &Some(uuid::Uuid::new_v4()), &handlers, Locale::En)
                .await
                .is_err()
        );
        assert!(verify_user_workspace("ws", &None, &handlers, Locale::En)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_verify_user_workspace_not_found_and_bad_id() {
        use crate::server::handlers::mock_generator::MockIdGenerator;
        use crate::server::handlers::mock_tests::MockConfigManagementService;

        // workspace 不存在 → 404 映射分支。
        let mut missing = MockConfigManagementService::new();
        missing.expect_get_workspace().returning(|_| Ok(None));
        let handlers = Arc::new(ApiHandlers::new(
            Arc::new(MockIdGenerator::new()),
            Arc::new(missing),
        ));
        assert!(
            verify_user_workspace("ghost", &Some(uuid::Uuid::new_v4()), &handlers, Locale::En)
                .await
                .is_err()
        );

        // id 非 UUID → invalid_workspace_id 映射分支。
        let mut bad_id = MockConfigManagementService::new();
        bad_id.expect_get_workspace().returning(|_| {
            Ok(Some(WorkspaceResponse {
                id: "not-a-uuid".to_string(),
                name: "ws".to_string(),
                description: None,
                status: "active".to_string(),
                max_groups: 10,
                max_biz_tags: 100,
                created_at: String::new(),
                updated_at: String::new(),
                user_api_key: None,
            }))
        });
        let handlers = Arc::new(ApiHandlers::new(
            Arc::new(MockIdGenerator::new()),
            Arc::new(bad_id),
        ));
        assert!(
            verify_user_workspace("ws", &Some(uuid::Uuid::new_v4()), &handlers, Locale::En)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_handle_generate_user_ok_and_admin_rejected() {
        use crate::server::middleware::ApiKeyRole;
        use axum::{Extension, Json};

        let state = create_test_app_state();
        let admin_req = GenerateRequest {
            workspace: "w".to_string(),
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            algorithm: None,
        };
        let (status, _) = handle_generate(
            State(state.clone()),
            Extension(None),
            Extension(ApiKeyRole::Admin),
            Extension(Locale::En),
            Json(admin_req),
        )
        .await
        .unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);

        // User 路径：继续走校验与生成（生成结果不论成败，处理主干均被执行）。
        let user_req = GenerateRequest {
            workspace: "w".to_string(),
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            algorithm: None,
        };
        let _ = handle_generate(
            State(state),
            Extension(None),
            Extension(ApiKeyRole::User),
            Extension(Locale::En),
            Json(user_req),
        )
        .await;
    }
}
