// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! sdforge `#[forge]` 多协议封装示例（Kit 范式）：只用 `nebulaid::sdk`
//! 公开面，零 DB 零网络。
//!
//! 运行：
//! ```bash
//! cargo run --package nebulaid --example sdk_server --features sdk,http
//! # 另开终端：
//! curl -X POST http://127.0.0.1:3000/generate \
//!      -H 'Content-Type: application/json' \
//!      -d '{"workspace":"ws","group":"g","biz_tag":"order"}'
//! curl -X POST http://127.0.0.1:3000/generate/batch \
//!      -H 'Content-Type: application/json' \
//!      -d '{"workspace":"ws","group":"g","biz_tag":"order","size":5}'
//! curl http://127.0.0.1:3000/api-docs/openapi.json
//! ```

// `t!` 经 #[macro_use] 拉入文本作用域（同 src/main.rs），取默认 locale 翻译。
#[macro_use]
extern crate nebulaid;

use std::sync::{Arc, OnceLock};

use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use nebulaid::core::Config;
use nebulaid::sdk::{IdGenerator, NebulaIdKitBuilder};
use sdforge::core::Registration;
use sdforge::prelude::*;
use sdforge::serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 初始化 sdforge 插件（HTTP/MCP/WebSocket inventory 提交）；须在构建
/// axum 路由前调用一次，否则 `#[forge]` 经 inventory 注册的路由会被链接器裁剪。
fn init_sdforge() -> sdforge::PluginCounts {
    sdforge::init_all_plugins()
}

/// 把 inventory 中注册的 `#[forge]` HTTP 路由合并进基础 Router。
fn merge_sdforge_routes(router: Router) -> Router {
    let mut router = router;
    for reg in sdforge::inventory::iter::<sdforge::http::RouteRegistration> {
        let route = reg.create();
        router = router.route(route.path(), route.handler().clone());
    }
    router
}

/// 进程级共享 `IdGenerator` handle：`#[forge]` 处理器为自由函数，经此静态
/// 句柄访问生成能力；缓存 handle 而非整个 Kit，避免生成热路径经 AsyncKit 读锁。
static GENERATOR: OnceLock<Arc<IdGenerator>> = OnceLock::new();

fn generator() -> &'static Arc<IdGenerator> {
    GENERATOR
        .get()
        .expect(&t!("error.sdk.generator_not_initialized"))
}

/// 把 `CoreError` 映射为 sdforge `ApiError`（500）。
///
/// 对外只给概要文案 + 一次性 `error_id`，细节经 `tracing::warn!` 留在服务端
/// 日志：`CoreError::Display` 会带出时钟回拨、数据库/驱动等内部状态，回显即
/// 泄露；`error_id` 同时出现在日志与响应，使排障可关联而不暴露原因。
fn to_api_error(e: nebulaid::core::CoreError) -> ApiError {
    let error_id = Uuid::now_v7().to_string();
    tracing::warn!(error = %e, error_id = %error_id, "embedded sdk request failed");
    ApiError::internal_error("ID generation failed, please retry", error_id)
}

/// 单条生成请求体。
#[derive(Debug, Clone, Deserialize)]
pub struct SdkGenerateRequest {
    pub workspace: String,
    pub group: String,
    pub biz_tag: String,
}

/// 单条生成响应体。
#[derive(Debug, Serialize)]
pub struct SdkGenerateResponse {
    pub id: String,
}

/// 批量生成请求体。
#[derive(Debug, Clone, Deserialize)]
pub struct SdkBatchGenerateRequest {
    pub workspace: String,
    pub group: String,
    pub biz_tag: String,
    pub size: usize,
}

/// 批量生成响应体。
#[derive(Debug, Serialize)]
pub struct SdkBatchGenerateResponse {
    pub ids: Vec<String>,
    pub count: usize,
}

/// POST /generate —— 生成单个 ID。
#[forge(
    name = "sdk_generate",
    version = "v1",
    path = "/generate",
    method = "POST",
    no_prefix = true,
    tool_name = "sdk_generate",
    description = "Generate a single ID via the embedded SDK (snowflake, zero DB)"
)]
async fn sdk_generate(req: SdkGenerateRequest) -> Result<SdkGenerateResponse, ApiError> {
    let id = generator()
        .generate(&req.workspace, &req.group, &req.biz_tag)
        .await
        .map_err(to_api_error)?;
    Ok(SdkGenerateResponse { id: id.to_string() })
}

/// POST /generate/batch —— 批量生成 ID。
#[forge(
    name = "sdk_batch_generate",
    version = "v1",
    path = "/generate/batch",
    method = "POST",
    no_prefix = true,
    tool_name = "sdk_batch_generate",
    description = "Generate IDs in batch via the embedded SDK (snowflake, zero DB)"
)]
async fn sdk_batch_generate(
    req: SdkBatchGenerateRequest,
) -> Result<SdkBatchGenerateResponse, ApiError> {
    let batch = generator()
        .batch_generate(&req.workspace, &req.group, &req.biz_tag, req.size)
        .await
        .map_err(to_api_error)?;
    let ids = batch
        .ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    let count = ids.len();
    Ok(SdkBatchGenerateResponse { ids, count })
}

/// GET /api-docs/openapi.json —— `#[forge]` 注册路由自动收集的 OpenAPI 文档。
///
/// 不用 `sdforge::swagger_ui_router`（依赖 nebulaid 未启用的 `docs` 特性），
/// 直接以 axum 路由提供 `sdforge::openapi::generate_openapi_spec()` 的 spec。
async fn serve_openapi_json() -> impl IntoResponse {
    Json(sdforge::openapi::generate_openapi_spec())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 默认算法须为 snowflake：Config::default() 的 segment 需数据库，未注入会被拒绝。
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();
    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = Arc::new(kit.id_generator()?);
    GENERATOR
        .set(generator)
        .map_err(|_| "generator already initialized")?;

    let counts = init_sdforge();
    println!(
        "sdforge plugins initialized: {} http route(s)",
        counts.routes
    );

    let app: Router = merge_sdforge_routes(Router::new())
        .route("/api-docs/openapi.json", get(serve_openapi_json));

    let addr: std::net::SocketAddr = "127.0.0.1:3000".parse()?;
    println!("sdk_server listening on http://{addr}");
    println!("  {}", t!("log.example.sdk_server_routes_generate"));
    println!("  {}", t!("log.example.sdk_server_routes_batch"));
    println!("  {}", t!("log.example.sdk_server_routes_openapi"));

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
