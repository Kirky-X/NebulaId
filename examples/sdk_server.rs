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

//! sdforge `#[forge]` 多协议封装示例（wiring T014）。
//!
//! 用 sdforge 的 `#[forge]` 属性宏把嵌入式 [`NebulaIdClient`] 的
//! `generate` / `batch_generate` 声明为 HTTP 端点（`POST /generate`、
//! `POST /generate/batch`），并由 sdforge 自动产出 OpenAPI 文档
//! （`GET /api-docs/openapi.json`）。纯算法（snowflake），零 DB 零网络。
//!
//! 本示例只依赖 `nebulaid::sdk` 公开面（R-sdk-003）：sdforge 插件初始化与
//! 路由合并直接内联如下，不复用 `server` 模块的内部装配知识。
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

use std::sync::{Arc, OnceLock};

use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use nebulaid::core::Config;
use nebulaid::sdk::{NebulaIdClient, NebulaIdClientBuilder};
use sdforge::core::Registration;
use sdforge::prelude::*;
use sdforge::serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 初始化 sdforge 插件（HTTP/MCP/WebSocket 等 inventory 提交）。
///
/// 必须在构建 axum 路由前调用一次，否则 `#[forge]` 经 inventory 注册的路由
/// 会被链接器裁剪。
///
/// 内联而非复用 `nebulaid::server::sdforge_adapter`：R-sdk-003 要求示例只用
/// sdk 公开面，嵌入方读这段代码即可照抄，不需要了解服务端模块的内部装配。
fn init_sdforge() -> sdforge::PluginCounts {
    sdforge::init_all_plugins()
}

/// 把 inventory 中注册的 `#[forge]` HTTP 路由合并进基础 Router。
///
/// 每个 `RouteRegistration::create()` 产出一个 `HttpRoute`，其 `path()` 与
/// `handler()`（`MethodRouter`）经 `Router::route` 挂载。
fn merge_sdforge_routes(router: Router) -> Router {
    let mut router = router;
    for reg in sdforge::inventory::iter::<sdforge::http::RouteRegistration> {
        let route = reg.create();
        router = router.route(route.path(), route.handler().clone());
    }
    router
}

/// 进程级共享客户端：`#[forge]` 处理器为自由函数，经此静态句柄访问客户端。
static CLIENT: OnceLock<Arc<NebulaIdClient>> = OnceLock::new();

fn client() -> &'static Arc<NebulaIdClient> {
    CLIENT
        .get()
        .expect("NebulaIdClient 未初始化（main 应先调用 build 并 set）")
}

/// 把 `CoreError` 映射为 sdforge `ApiError`（500 内部错误）。
///
/// 对外**只**给稳定的概要文案 + 一次性 `error_id`；内部错误细节经
/// `tracing::warn!` 留在服务端日志。`CoreError` 的 `Display` 会带出实现细节
/// （如 `ClockMovedBackward` 的内部 `last_timestamp`、数据库/驱动错误文本），
/// 原样回显等于向调用方泄露内部状态——示例是嵌入方照抄的模板，必须示范
/// "细节进日志、摘要出网关" 的边界。`error_id` 同时出现在日志与响应的
/// error_id 字段里，让排障可关联而不暴露原因。
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
    description = "通过嵌入式 SDK 生成单个 ID（snowflake，零 DB）"
)]
async fn sdk_generate(req: SdkGenerateRequest) -> Result<SdkGenerateResponse, ApiError> {
    let id = client()
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
    description = "通过嵌入式 SDK 批量生成 ID（snowflake，零 DB）"
)]
async fn sdk_batch_generate(
    req: SdkBatchGenerateRequest,
) -> Result<SdkBatchGenerateResponse, ApiError> {
    let batch = client()
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

/// GET /api-docs/openapi.json —— 由 `#[forge]` 注册路由自动收集的 OpenAPI 文档。
///
/// 不使用 `sdforge::swagger_ui_router`（其依赖 sdforge `docs` 特性，
/// nebulaid 未启用）；此处直接以 axum 路由提供
/// `sdforge::openapi::generate_openapi_spec()` 生成的 spec。
async fn serve_openapi_json() -> impl IntoResponse {
    Json(sdforge::openapi::generate_openapi_spec())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 构建嵌入式客户端：默认算法 snowflake（纯算法，零 DB 零网络）。
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();
    let client = NebulaIdClientBuilder::new(config).build().await?;
    CLIENT
        .set(Arc::new(client))
        .map_err(|_| "client already initialized")?;

    // 初始化 sdforge 插件并合并 #[forge] 注册的 HTTP 路由（装配函数见文件
    // 上方），再挂载 Swagger/OpenAPI 路由。
    let counts = init_sdforge();
    println!(
        "sdforge plugins initialized: {} http route(s)",
        counts.routes
    );

    let app: Router = merge_sdforge_routes(Router::new())
        .route("/api-docs/openapi.json", get(serve_openapi_json));

    let addr: std::net::SocketAddr = "127.0.0.1:3000".parse()?;
    println!("sdk_server listening on http://{addr}");
    println!("  POST /generate            生成单个 ID");
    println!("  POST /generate/batch      批量生成 ID");
    println!("  GET  /api-docs/openapi.json  OpenAPI 文档");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
