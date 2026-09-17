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

use crate::core::types::CoreError;
use crate::server::handlers::helpers::authorize_workspace_access;
use crate::server::handlers::ApiHandlers;
use crate::server::middleware::api_key_auth::{parse_authorization_header, ApiKeyAuth, ApiKeyRole};
use crate::server::models::{BatchGenerateRequest, GenerateRequest, ParseRequest};
use async_trait::async_trait;
use sdforge::tonic::{Request, Response, Status};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

// Use pre-generated proto modules
use crate::server::proto::nebula::id::v1;

use v1::nebula_id_service_server::NebulaIdService;
use v1::{
    BatchGenerateRequest as GrpcBatchGenerateRequest,
    BatchGenerateResponse as GrpcBatchGenerateResponse, BatchGenerateStreamRequest,
    BatchGenerateStreamResponse, GenerateRequest as GrpcGenerateRequest,
    GenerateResponse as GrpcGenerateResponse, HealthCheckRequest, HealthCheckResponse,
    ParseRequest as GrpcParseRequest, ParseResponse as GrpcParseResponse,
};

pub struct GrpcServer {
    handlers: Arc<ApiHandlers>,
    /// 认证器。None = 不启用（既有测试/内网部署语义）。
    auth: Option<Arc<ApiKeyAuth>>,
}

impl GrpcServer {
    pub fn new(handlers: Arc<ApiHandlers>) -> Self {
        Self {
            handlers,
            auth: None,
        }
    }

    /// 启用 API key 认证。启用后每个 RPC 入口先经
    /// [`Self::authenticate`] 校验 `authorization` metadata。
    pub fn with_auth(handlers: Arc<ApiHandlers>, auth: Arc<ApiKeyAuth>) -> Self {
        Self {
            handlers,
            auth: Some(auth),
        }
    }

    /// 单点认证：校验 `authorization` metadata（Basic/ApiKey 双格式），成功时
    /// 将 workspace_id 与角色注入 request extensions。覆盖全部 RPC（含双向流
    /// —— request-init 先于流消费被校验）。失败映射（规格）
    /// - 缺失/格式无效/凭证无效 → `Status::unauthenticated`
    /// - key 存在但被禁用或已过期 → `Status::permission_denied`
    /// - 同 peer 认证失败超阈值（5 分钟 10 次）→ `Status::resource_exhausted`
    ///   （T012：与 HTTP 中间件共享同一失败桶，429 对应 ResourceExhausted）
    /// - `auth.enabled=false` → 放行并记 `auth_disabled_request` 审计日志
    ///   （对齐 HTTP Anonymous 语义）
    ///
    /// 注：设计稿原定 tonic `with_interceptor`，但 Interceptor::call 是同步
    /// 签名而凭证校验必须异步查库（Argon2id + DB），在拦截器内 block_on 有
    /// 运行时风险，故改为各 RPC 入口一行调用本助手 —— 单点实现不变。
    ///
    /// 对端 IP 取自 tonic 传输注入的 `TcpConnectInfo`（TLS 下为
    /// `TlsConnectInfo<TcpConnectInfo>`），[`Request::remote_addr`] 已封装
    /// 两种情况；限流桶与审计日志均按直连 IP 归因，不可被头部伪造。
    pub(crate) async fn authenticate<T>(&self, request: Request<T>) -> Result<Request<T>, Status> {
        let Some(auth) = self.auth.as_ref() else {
            return Ok(request);
        };
        // 对端 IP 提前解析：禁用放行的审计行与失败拒绝行共用同一取值，
        // 避免为一条日志重复读一次扩展。
        let client_ip = peer_ip(&request);

        if !auth.is_enabled() {
            // `auth.enabled=false` 的放行不是「无事件」：HTTP 侧同语义
            // （api_key_auth.rs 的 auth_disabled_request）会留一条 warn，gRPC
            // 侧静默放行会让降级部署下的流量在审计里隐身。文案与 event 字段名
            // 沿用 HTTP 侧既有 locale 键，跨传输可按同一 event 检索。
            tracing::warn!(
                event = "auth_disabled_request",
                client_ip = %client_ip,
                "{}",
                t!("log.server.middleware.api_key_auth.auth_disabled_request")
            );
            return Ok(request);
        }

        // T012 — 与 HTTP 中间件同源：先查失败桶，超限直接拒收，
        // 不再消耗解析/校验成本。桶体（阈值、窗口、容量阀）全部在
        // `ApiKeyAuth` 内，两条传输线共享同一计数器。
        if !auth.check_auth_failure_rate(&client_ip) {
            return Err(Status::resource_exhausted(
                "Too many authentication attempts. Please try again later.",
            ));
        }

        let Some(value) = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
        else {
            return Err(reject(
                auth,
                &client_ip,
                "",
                "missing_authorization",
                &t!("log.server.middleware.api_key_auth.missing_auth_header"),
                Status::unauthenticated("missing authorization metadata"),
            ));
        };
        let Some((key_id, key_secret)) = parse_authorization_header(value) else {
            return Err(reject(
                auth,
                &client_ip,
                "",
                "unsupported_auth_format",
                &t!("log.server.middleware.api_key_auth.unsupported_auth_format"),
                Status::unauthenticated("invalid authorization format"),
            ));
        };

        match auth.validate_key(&key_id, &key_secret).await {
            Some(auth) => {
                let mut request = request;
                request.extensions_mut().insert(auth.workspace_id);
                request.extensions_mut().insert(auth.role);
                Ok(request)
            }
            // 校验 miss：回查 key 行区分「凭证无效」与「key 被禁用/已过期」
            None => Err(classify_miss(auth, &key_id, &client_ip).await),
        }
    }
}

/// 取 gRPC 对端 IP（直连地址，不可被头部伪造）。非 TCP 传输（如单测里直接
/// 调用 trait 方法）取不到地址，兜底 `"unknown"` —— 与 HTTP 侧
/// `api_key_auth.rs::get_client_ip` 的兜底语义一致。
fn peer_ip<T>(request: &Request<T>) -> String {
    request
        .remote_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// 从请求扩展中提取资源级授权所需的身份（T011）。
///
/// 认证启用且校验成功时，[`GrpcServer::authenticate`] 必然注入
/// `Option<uuid::Uuid>`（workspace_id）与 `ApiKeyRole` 两个扩展。二者缺失
/// 意味着认证未装配（`GrpcServer::new`，内网部署）或 `auth.enabled=false`
/// （降级放行）——两条路径都返回 `None`，由 [`authorize_namespace`] 跳过
/// 资源级判定，维持既有 e2e 钉住的降级语义。
fn auth_identity<T>(request: &Request<T>) -> Option<(ApiKeyRole, Option<uuid::Uuid>)> {
    let role = request.extensions().get::<ApiKeyRole>().cloned()?;
    let workspace_id = request.extensions().get::<Option<uuid::Uuid>>().copied()?;
    Some((role, workspace_id))
}

/// generate 系接口的资源级授权判定（T011）。
///
/// 与 HTTP 侧 `verify_user_role` + `verify_user_workspace` 同源：
///
/// - Admin key → `Status::permission_denied`（HTTP `verify_user_role` 的
///   `admin_cannot_perform` 403 语义；generate 类接口仅面向 User）。
/// - User key 的 namespace 经 handlers 既有 config_service
///   （`repository::get_workspace_by_name`，不改仓储层）反查 UUID；
///   反查不到 → `Status::not_found`，反查基础设施失败 → `Status::internal`
///   （固定文案，与 HTTP 侧 500 泛化消息同语义，不透出内部错误）。
/// - 反查成功后交由共享授权函数 [`authorize_workspace_access`]（T010）做
///   角色租户判定：跨租户 → `Status::permission_denied`。
///
/// 认证未装配 / 禁用（`identity = None`）时跳过判定放行：与既有
/// `bypasses_when_auth_disabled` 等 e2e 钉住的降级语义一致。
async fn authorize_namespace(
    handlers: &ApiHandlers,
    identity: Option<(ApiKeyRole, Option<uuid::Uuid>)>,
    namespace: &str,
) -> Result<(), Status> {
    let Some((role, key_workspace_id)) = identity else {
        return Ok(());
    };

    // Admin：generate 类接口仅面向 User（对齐 HTTP verify_user_role）。
    if role == ApiKeyRole::Admin {
        return Err(Status::permission_denied(
            t!("api.error.admin_cannot_perform").to_string(),
        ));
    }

    // User 角色但 key 无租户绑定：无法证明归属，fail-closed。
    let Some(key_workspace_id) = key_workspace_id else {
        tracing::warn!(
            event = "authz_user_key_without_workspace",
            "user-role key carries no workspace binding; denying generate access"
        );
        return Err(Status::permission_denied(
            t!("api.error.workspace_mismatch").to_string(),
        ));
    };

    // namespace → workspace 反查（NotFound / 基础设施失败两条出口分开处理）。
    let workspace = match handlers.get_workspace(namespace).await {
        Ok(Some(ws)) => ws,
        Ok(None) => {
            return Err(Status::not_found(
                t!("api.error.workspace_not_found").to_string(),
            ));
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                namespace = %namespace,
                "workspace lookup failed during gRPC authorization"
            );
            return Err(Status::internal("internal error"));
        }
    };

    let target_workspace_id = match uuid::Uuid::parse_str(&workspace.id) {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(
                error = %e,
                workspace_id = %workspace.id,
                "workspace row carries a non-UUID id"
            );
            return Err(Status::internal("internal error"));
        }
    };

    match authorize_workspace_access(&role, key_workspace_id, target_workspace_id).await {
        Ok(()) => Ok(()),
        // User 请求他人 namespace → permission_denied（HTTP 403
        // workspace_mismatch 同语义）。
        Err(CoreError::WorkspaceDisabled(_)) => Err(Status::permission_denied(
            t!("api.error.workspace_mismatch").to_string(),
        )),
        Err(e) => {
            // 其余拒绝变体（Anonymous 等，gRPC 正常路径不会注入）按
            // unauthenticated 处理，fail-closed。
            tracing::warn!(error = %e, "workspace authorization denied");
            Err(Status::unauthenticated(
                t!("api.error.auth_required").to_string(),
            ))
        }
    }
}

/// 认证拒绝的统一出口：先记入与 HTTP 中间件共享的按 IP 失败桶（T012），
/// 再打一条与 HTTP 侧同 schema 的 `auth_failure` 审计日志（`reason` 机器
/// 可读、`client_ip`、掩码 `key_id_prefix`），最后返回调用方给定的 Status。
/// 三者缺一不可 —— 只返 Status 会让 gRPC 侧的暴力猜 key 行为既绕过限流、
/// 又在审计里隐身。
///
/// 日志文案复用 `log.server.middleware.api_key_auth.*` 既有键：语义完全对应
/// （缺少 authorization / 不支持的格式 / 无效凭据），新增 grpc 专属键只会在
/// 两个 locale 里造出同义重复条目。
fn reject(
    auth: &ApiKeyAuth,
    client_ip: &str,
    key_id_prefix: &str,
    reason: &str,
    log_message: &str,
    status: Status,
) -> Status {
    // 与 HTTP `unauthorized_response` 同源：每次认证拒绝都入桶，
    // 供下一次请求的 `check_auth_failure_rate` 累计判定。
    auth.record_auth_failure(client_ip);
    tracing::warn!(
        event = "auth_failure",
        reason = reason,
        client_ip = %client_ip,
        key_id_prefix = %key_id_prefix,
        "{}",
        log_message
    );
    status
}

/// `validate_key` miss 的判因结果。用显式枚举而非字符串匹配，避免把
/// 「哪种失败给哪个 code」这一确定性决策写成隐式约定。
enum KeyMiss {
    /// key 行存在但 `enabled = false`
    Disabled,
    /// key 行存在且启用，但绝对过期时间已过
    Expired,
    /// key 行存在且有效 ⇒ 是 secret 不符
    BadSecret,
    /// key 行不存在，或判因回查本身失败
    Unknown,
}

impl KeyMiss {
    fn reject(&self, auth: &ApiKeyAuth, client_ip: &str, key_id: &str) -> Status {
        let (reason, status) = match self {
            // 身份可识别但被授权层拒绝 ⇒ permission_denied
            Self::Disabled => (
                "key_disabled",
                Status::permission_denied(CoreError::ApiKeyDisabled.to_string()),
            ),
            Self::Expired => (
                "key_expired",
                Status::permission_denied(CoreError::ApiKeyExpired.to_string()),
            ),
            // 凭证本身无效 ⇒ unauthenticated（文案不区分 key 是否存在，
            // 避免把「哪些 key_id 有效」泄露给探测者）
            Self::BadSecret => (
                "bad_secret",
                Status::unauthenticated("invalid or unknown api key"),
            ),
            Self::Unknown => (
                "unknown_key",
                Status::unauthenticated("invalid or unknown api key"),
            ),
        };
        reject(
            auth,
            client_ip,
            &key_id.chars().take(8).collect::<String>(),
            reason,
            &t!("log.server.middleware.api_key_auth.invalid_credentials"),
            status,
        )
    }
}

/// 回查 key 行完成判因（规格）。HTTP 侧无此分类（一律 401），
/// 故这里是唯一实现，不构成第二份重复逻辑；禁用/过期文案复用
/// `CoreError::ApiKeyDisabled/ApiKeyExpired` 的 i18n 条目而非另写字面量。
///
/// 回查失败（DB 异常）时保守按 `unauthenticated` 拒绝：既不能确认 key 状态
/// 就不得放行，也不能凭猜给 `permission_denied` 谎报语义；同时以 error 级
/// 显性上报，避免 DB 故障被伪装成「凭证无效」。
async fn classify_miss(auth: &ApiKeyAuth, key_id: &str, client_ip: &str) -> Status {
    let info = match auth.repo.get_api_key_by_id(key_id).await {
        Ok(info) => info,
        Err(e) => {
            tracing::error!(
                error = %e,
                event = "auth_failure",
                reason = "key_state_query_failed",
                client_ip = %client_ip,
                "failed to load api key state for auth failure classification"
            );
            return KeyMiss::Unknown.reject(auth, client_ip, key_id);
        }
    };

    let miss = match info {
        None => KeyMiss::Unknown,
        Some(info) => {
            if !info.enabled {
                KeyMiss::Disabled
            } else if info
                .expires_at
                .is_some_and(|at| at < chrono::Utc::now().naive_utc())
            {
                KeyMiss::Expired
            } else {
                KeyMiss::BadSecret
            }
        }
    };
    miss.reject(auth, client_ip, key_id)
}

#[async_trait]
impl NebulaIdService for GrpcServer {
    type BatchGenerateStreamStream = ReceiverStream<Result<BatchGenerateStreamResponse, Status>>;

    async fn generate(
        &self,
        request: Request<GrpcGenerateRequest>,
    ) -> Result<Response<GrpcGenerateResponse>, Status> {
        let request = self.authenticate(request).await?;
        // T011 资源级授权：namespace 在消息体内，into_inner 前按借用读取。
        authorize_namespace(
            &self.handlers,
            auth_identity(&request),
            &request.get_ref().namespace,
        )
        .await?;
        let req = request.into_inner();
        let tag = req.tag.clone();

        let generate_req = GenerateRequest {
            workspace: req.namespace,
            group: tag.clone(),
            biz_tag: tag,
            algorithm: None,
        };

        match self.handlers.generate(generate_req).await {
            Ok(resp) => {
                let timestamp = resp.timestamp.parse().unwrap_or(0);
                Ok(Response::new(GrpcGenerateResponse {
                    id: resp.id,
                    timestamp,
                    sequence: 0,
                    worker_id: 0,
                    algorithm: resp.algorithm,
                }))
            }
            Err(e) => Err(Status::internal(format!("{}", e))),
        }
    }

    async fn batch_generate(
        &self,
        request: Request<GrpcBatchGenerateRequest>,
    ) -> Result<Response<GrpcBatchGenerateResponse>, Status> {
        let request = self.authenticate(request).await?;
        // T011 资源级授权：与 generate 同一判定点（into_inner 前）。
        authorize_namespace(
            &self.handlers,
            auth_identity(&request),
            &request.get_ref().namespace,
        )
        .await?;
        let req = request.into_inner();
        let tag = req.tag.clone();

        tracing::info!(
            "{}",
            t!("log.server.grpc.batch_generate_received", count = req.count)
        );

        // Validate batch size（错误消息与 HTTP 同源 i18n）
        if req.count == 0 {
            tracing::warn!(
                "{}",
                t!("log.server.grpc.batch_size_validation_failed_zero")
            );
            return Err(Status::invalid_argument(
                t!("api.error.handlers.id_handlers.batch_size_zero").to_string(),
            ));
        }
        // 上限唯一来源 = config.batch_generate.max_batch_size
        let max_batch_size = self.handlers.get_config_service().get_batch_max_size() as usize;
        if req.count > max_batch_size as i32 {
            tracing::warn!(
                "{}",
                t!(
                    "log.server.grpc.batch_size_validation_failed_exceeds_max",
                    count = req.count,
                    max = max_batch_size
                )
            );
            return Err(Status::invalid_argument(
                t!(
                    "api.error.handlers.id_handlers.batch_size_exceeds_max",
                    size = req.count,
                    max = max_batch_size
                )
                .to_string(),
            ));
        }

        tracing::info!(
            "{}",
            t!(
                "log.server.grpc.batch_size_validation_passed",
                count = req.count
            )
        );

        let batch_req = BatchGenerateRequest {
            workspace: req.namespace,
            group: tag.clone(),
            biz_tag: tag,
            size: Some(req.count as usize),
            algorithm: None,
        };

        match self.handlers.batch_generate(batch_req).await {
            Ok(resp) => {
                let timestamp = resp.timestamp.parse().unwrap_or(0);
                let ids = resp
                    .ids
                    .into_iter()
                    .map(|id| GrpcGenerateResponse {
                        id,
                        timestamp,
                        sequence: 0,
                        worker_id: 0,
                        algorithm: resp.algorithm.clone(),
                    })
                    .collect();

                Ok(Response::new(GrpcBatchGenerateResponse { ids }))
            }
            Err(e) => Err(Status::internal(format!("{}", e))),
        }
    }

    async fn batch_generate_stream(
        &self,
        request: Request<sdforge::tonic::Streaming<BatchGenerateStreamRequest>>,
    ) -> Result<Response<Self::BatchGenerateStreamStream>, Status> {
        // request-init 先于流消费被校验，流式入口同样受保护
        let request = self.authenticate(request).await?;
        // T011 资源级授权：身份在 into_inner 前提取；namespace 是逐项字段，
        // 逐项判定在流任务内完成，拒绝以 Status 终止流（Admin/跨租户均如此）。
        let identity = auth_identity(&request);
        let mut stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);

        let handlers = self.handlers.clone();

        tokio::spawn(async move {
            while let Some(req) = stream.next().await {
                match req {
                    Ok(stream_req) => {
                        // T011 资源级授权：逐项 namespace 判定，拒绝以 Status 终止流。
                        if let Err(status) =
                            authorize_namespace(&handlers, identity.clone(), &stream_req.namespace)
                                .await
                        {
                            let _ = tx.send(Err(status)).await;
                            break;
                        }
                        let tag = stream_req.tag.clone();
                        let batch_req = BatchGenerateRequest {
                            workspace: stream_req.namespace,
                            group: tag.clone(),
                            biz_tag: tag,
                            size: Some(stream_req.count as usize),
                            algorithm: None,
                        };

                        match handlers.batch_generate(batch_req).await {
                            Ok(resp) => {
                                let timestamp = resp.timestamp.parse().unwrap_or(0);
                                for id in resp.ids {
                                    let stream_resp = BatchGenerateStreamResponse {
                                        id: Some(GrpcGenerateResponse {
                                            id,
                                            timestamp,
                                            sequence: 0,
                                            worker_id: 0,
                                            algorithm: resp.algorithm.clone(),
                                        }),
                                    };

                                    if tx.send(Ok(stream_resp)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = tx
                                    .send(Ok(BatchGenerateStreamResponse {
                                        id: Some(GrpcGenerateResponse {
                                            id: String::new(),
                                            timestamp: 0,
                                            sequence: 0,
                                            worker_id: 0,
                                            algorithm: format!("error: {}", e),
                                        }),
                                    }))
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Ok(BatchGenerateStreamResponse {
                                id: Some(GrpcGenerateResponse {
                                    id: String::new(),
                                    timestamp: 0,
                                    sequence: 0,
                                    worker_id: 0,
                                    algorithm: format!("stream error: {}", e),
                                }),
                            }))
                            .await;
                        break;
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn parse(
        &self,
        request: Request<GrpcParseRequest>,
    ) -> Result<Response<GrpcParseResponse>, Status> {
        let request = self.authenticate(request).await?;
        let req = request.into_inner();

        let parse_req = ParseRequest {
            id: req.id.clone(),
            workspace: String::new(),
            group: String::new(),
            biz_tag: String::new(),
            algorithm: String::new(),
        };

        match self.handlers.parse(parse_req).await {
            Ok(resp) => {
                let timestamp = resp.timestamp.parse().unwrap_or(0);
                let metadata: HashMap<String, String> = vec![
                    ("timestamp".to_string(), resp.metadata.timestamp.to_string()),
                    (
                        "datacenter_id".to_string(),
                        resp.metadata.datacenter_id.to_string(),
                    ),
                    ("worker_id".to_string(), resp.metadata.worker_id.to_string()),
                    ("sequence".to_string(), resp.metadata.sequence.to_string()),
                    ("algorithm".to_string(), resp.metadata.algorithm),
                    ("biz_tag".to_string(), resp.metadata.biz_tag),
                ]
                .into_iter()
                .collect();

                Ok(Response::new(GrpcParseResponse {
                    id: resp.original_id,
                    timestamp,
                    sequence: resp.metadata.sequence as i32,
                    worker_id: resp.metadata.worker_id as i32,
                    algorithm: resp.algorithm,
                    metadata,
                }))
            }
            Err(e) => Err(Status::invalid_argument(format!("{}", e))),
        }
    }

    async fn health_check(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        // health_check 同样纳入认证（设计 覆盖全部 5 个 RPC）。
        // 编排探针请改用 HTTP 端口 /health（公开端点）。
        // 健康状态不依赖租户身份，认证结果只需通过即可，故丢弃 request。
        self.authenticate(request).await?;
        let health = self.handlers.health().await;
        let status = if health.status == crate::server::models::HealthStatus::Healthy {
            v1::health_check_response::ServingStatus::Serving
        } else {
            v1::health_check_response::ServingStatus::NotServing
        };

        Ok(Response::new(HealthCheckResponse {
            status: status as i32,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::algorithm::AlgorithmRouter;
    use crate::core::config::Config;
    use crate::server::config::management::{ConfigManagementService, ConfigManager};
    use crate::server::config::HotReloadConfig;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use std::sync::Arc;

    /// Build a GrpcServer wired to a MockIdGenerator + ConfigManager.
    fn create_test_grpc_server() -> GrpcServer {
        let config = Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = Arc::new(AlgorithmRouter::new(config, None));
        let config_service: Arc<dyn ConfigManagementService> =
            Arc::new(ConfigManager::new(hot_config, algorithm_router));
        let id_generator: Arc<dyn crate::core::algorithm::IdGenerator> =
            Arc::new(MockIdGenerator::new());
        let handlers = Arc::new(ApiHandlers::new(id_generator, config_service));
        GrpcServer::new(handlers)
    }

    // ===== GrpcServer::new =====

    #[test]
    fn test_grpc_server_new() {
        let server = create_test_grpc_server();
        // Smoke test: server can be constructed without panic.
        let _ = server;
    }

    // ===== generate =====

    #[tokio::test]
    async fn test_generate_success() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            metadata: Default::default(),
        });
        let resp = server.generate(req).await;
        assert!(resp.is_ok(), "generate should succeed: {:?}", resp);
        let inner = resp.unwrap().into_inner();
        assert!(!inner.id.is_empty());
        assert_eq!(inner.algorithm, "segment");
        assert_eq!(inner.sequence, 0);
        assert_eq!(inner.worker_id, 0);
    }

    #[tokio::test]
    async fn test_generate_empty_namespace_returns_internal_error() {
        // MockIdGenerator returns InvalidInput when workspace is empty.
        let server = create_test_grpc_server();
        let req = Request::new(GrpcGenerateRequest {
            namespace: String::new(),
            tag: "test-tag".to_string(),
            metadata: Default::default(),
        });
        let resp = server.generate(req).await;
        assert!(resp.is_err());
        let err = resp.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::Internal);
    }

    #[tokio::test]
    async fn test_generate_maps_namespace_and_tag() {
        // Verify that `namespace` is mapped to `workspace` and `tag` is mapped
        // to both `group` and `biz_tag` (per the handler's GenerateRequest
        // construction).
        let server = create_test_grpc_server();
        let req = Request::new(GrpcGenerateRequest {
            namespace: "mapped-ns".to_string(),
            tag: "mapped-tag".to_string(),
            metadata: Default::default(),
        });
        let resp = server.generate(req).await.unwrap().into_inner();
        // ID should be non-empty (MockIdGenerator generates u128 IDs).
        assert!(!resp.id.is_empty());
    }

    // ===== batch_generate =====

    #[tokio::test]
    async fn test_batch_generate_success() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 5,
            metadata: Default::default(),
        });
        let resp = server.batch_generate(req).await;
        assert!(resp.is_ok(), "batch_generate should succeed: {:?}", resp);
        let inner = resp.unwrap().into_inner();
        assert_eq!(inner.ids.len(), 5);
        for id in &inner.ids {
            assert!(!id.id.is_empty());
            assert_eq!(id.algorithm, "segment");
        }
    }

    #[tokio::test]
    async fn test_batch_generate_count_one_boundary() {
        // Lower boundary: count=1 should succeed.
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 1,
            metadata: Default::default(),
        });
        let resp = server.batch_generate(req).await.unwrap().into_inner();
        assert_eq!(resp.ids.len(), 1);
    }

    #[tokio::test]
    async fn test_batch_generate_count_100_boundary() {
        // Upper boundary: count=100 should succeed.
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 100,
            metadata: Default::default(),
        });
        let resp = server.batch_generate(req).await.unwrap().into_inner();
        assert_eq!(resp.ids.len(), 100);
    }

    #[tokio::test]
    async fn test_batch_generate_zero_count_returns_invalid_argument() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 0,
            metadata: Default::default(),
        });
        let err = server.batch_generate(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
        assert!(err.message().contains("zero"));
    }

    #[tokio::test]
    async fn test_batch_generate_exceeds_max_101_returns_invalid_argument() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 101,
            metadata: Default::default(),
        });
        let err = server.batch_generate(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
        assert!(err.message().contains("exceeds maximum"));
    }

    #[tokio::test]
    async fn test_batch_generate_huge_count_returns_invalid_argument() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 1000,
            metadata: Default::default(),
        });
        let err = server.batch_generate(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_batch_generate_empty_namespace_returns_internal_error() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: String::new(),
            tag: "test-tag".to_string(),
            count: 5,
            metadata: Default::default(),
        });
        let err = server.batch_generate(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::Internal);
    }

    #[tokio::test]
    async fn test_batch_generate_maps_namespace_and_tag() {
        // Verify namespace→workspace and tag→group/biz_tag mapping by
        // observing that a valid request succeeds (MockIdGenerator returns
        // Err only when workspace is empty).
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "mapped-ns".to_string(),
            tag: "mapped-tag".to_string(),
            count: 3,
            metadata: Default::default(),
        });
        let resp = server.batch_generate(req).await.unwrap().into_inner();
        assert_eq!(resp.ids.len(), 3);
        for id in &resp.ids {
            assert!(!id.id.is_empty());
        }
    }

    // ===== parse =====

    #[tokio::test]
    async fn test_parse_valid_numeric_id() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcParseRequest {
            id: "12345".to_string(),
        });
        let resp = server.parse(req).await;
        assert!(resp.is_ok(), "parse should succeed: {:?}", resp);
        let inner = resp.unwrap().into_inner();
        assert_eq!(inner.id, "12345");
        // metadata should contain timestamp, datacenter_id, worker_id, etc.
        assert!(inner.metadata.contains_key("timestamp"));
        assert!(inner.metadata.contains_key("algorithm"));
    }

    #[tokio::test]
    async fn test_parse_invalid_id_returns_invalid_argument() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcParseRequest {
            id: "not-a-valid-id".to_string(),
        });
        let err = server.parse(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_parse_empty_id_returns_invalid_argument() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcParseRequest { id: String::new() });
        let err = server.parse(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
    }

    // ===== health_check =====

    #[tokio::test]
    async fn test_health_check_returns_serving() {
        // MockIdGenerator.health_check() returns Healthy, so the gRPC
        // health check should report Serving.
        let server = create_test_grpc_server();
        let req = Request::new(HealthCheckRequest {
            service: String::new(),
        });
        let resp = server.health_check(req).await.unwrap().into_inner();
        assert_eq!(
            resp.status,
            v1::health_check_response::ServingStatus::Serving as i32
        );
    }

    // ===== peer_ip（认证失败按直连 IP 归因的前置能力）=====

    #[test]
    fn test_peer_ip_reads_connect_info_injected_by_tonic_transport() {
        // tonic 0.14 的 transport server 在 MakeSvc::call 里用 ConnectInfoLayer
        // 把 TcpConnectInfo 注入 request extensions。这里手工注入同一类型，
        // 验证 peer_ip 取的是该真实扩展（而非自造字段或元数据）。
        let mut req = Request::new(HealthCheckRequest {
            service: String::new(),
        });
        req.extensions_mut()
            .insert(sdforge::tonic::transport::server::TcpConnectInfo {
                local_addr: None,
                remote_addr: Some("203.0.113.7:5555".parse().unwrap()),
            });
        // 只取 IP：端口每次连接都变，归因桶必须按主机聚合
        assert_eq!(peer_ip(&req), "203.0.113.7");
    }

    #[test]
    fn test_peer_ip_falls_back_to_unknown_without_transport() {
        // 无 ConnectInfo（单测直接调用 trait 方法 / 非 TCP 传输）时兜底
        // "unknown"，与 HTTP 侧 get_client_ip 的兜底语义一致 —— 不伪造地址。
        let req = Request::new(HealthCheckRequest {
            service: String::new(),
        });
        assert_eq!(peer_ip(&req), "unknown");
    }
}
