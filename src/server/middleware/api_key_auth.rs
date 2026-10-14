// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use crate::core::database::{ApiKeyRepository, AuthenticatedKey};
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use base64::Engine;
#[cfg(not(feature = "garrison-auth"))]
use parking_lot::RwLock;
#[cfg(not(feature = "garrison-auth"))]
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
#[cfg(not(feature = "garrison-auth"))]
use std::time::Duration;
use std::time::Instant;

// Re-export ApiKeyRole locally for use in this module
pub use crate::core::database::ApiKeyRole;

/// 认证失败滑窗窗口（秒）与阈值：HTTP 429 与 gRPC ResourceExhausted 共享同一策略。
/// 阈值以 garrison `record_failure` 的 `new_count > max_attempts` 触发封禁为准，
/// 故 `max_attempts = MAX - 1`，第 `MAX` 次失败触发封禁。
const AUTH_FAILURE_WINDOW_SECS: u64 = 300;
const AUTH_FAILURE_MAX_FAILURES: u64 = 10;

/// garrison `BruteForceStrategy` 的 `max_attempts`（阈值减一）。
#[cfg(feature = "garrison-auth")]
const AUTH_FAILURE_MAX_ATTEMPTS: u32 = (AUTH_FAILURE_MAX_FAILURES - 1) as u32;

/// 封禁时长（秒），与窗口同值，避免封禁期长于计数窗口。
#[cfg(feature = "garrison-auth")]
const AUTH_FAILURE_LOCK_SECS: u64 = AUTH_FAILURE_WINDOW_SECS;

/// 剪裁构建（无 garrison-auth）进程内滑窗的 IP 跟踪上限。
#[cfg(not(feature = "garrison-auth"))]
const MAX_TRACKED_AUTH_FAILURE_IPS: usize = 10_000;

/// garrison-auth 构建的失败表键数上限（计数键 + 封禁键合计，约 25k IP × 4 键）。
/// InMemoryDao 无容量上限且过期键只惰性清理，键空间受攻击者控制，
/// 故经 [`crate::server::auth::BoundedDao`] 装饰器设界并按 FIFO 淘汰。
#[cfg(feature = "garrison-auth")]
const MAX_TRACKED_AUTH_FAILURE_KEYS: usize = 100_000;

#[derive(Clone)]
pub struct ApiKeyAuth {
    pub(crate) repo: Arc<dyn ApiKeyRepository>,
    pub(crate) enabled: bool,
    trusted_proxies: Vec<IpAddr>,
    /// garrison-auth：认证失败限速委托 `BruteForceStrategy`；HTTP 与 gRPC
    /// 共用同一实例即共享同一失败桶。
    #[cfg(feature = "garrison-auth")]
    brute_force: Arc<garrison::strategy::firewall::BruteForceStrategy>,
    /// 剪裁构建回退：进程内滑窗失败表（库不可用也不得去掉防爆破保护）。
    #[cfg(not(feature = "garrison-auth"))]
    auth_failures: Arc<RwLock<HashMap<String, Vec<Instant>>>>,
    /// 本仓 `AuthCache`（类型化进程内认证决策缓存）；`None` = 不缓存。
    #[cfg(feature = "garrison-auth")]
    cache: Option<Arc<crate::server::auth::AuthCache>>,
}

impl ApiKeyAuth {
    pub fn new(repo: Arc<dyn ApiKeyRepository>, enabled: bool) -> Self {
        Self {
            repo,
            enabled,
            trusted_proxies: Vec::new(),
            #[cfg(feature = "garrison-auth")]
            brute_force: Arc::new(Self::build_brute_force_strategy()),
            #[cfg(not(feature = "garrison-auth"))]
            auth_failures: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "garrison-auth")]
            cache: None,
        }
    }

    /// 将 client 标识映射为 garrison `FirewallContext` 接受的 IP 字符串。
    ///
    /// 合法 IP 原样透传；非法标识（gRPC 的 "unknown" 兜底、XFF 首段任意文本）
    /// 按哈希派生 ULA 段（`fc00::/7`）地址：同标识恒同桶、不伪造真实地址，
    /// 只作限流桶键，日志与审计仍输出原始 `client_ip`。
    #[cfg(feature = "garrison-auth")]
    fn client_ip_to_context_ip(client_ip: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        if client_ip.parse::<std::net::IpAddr>().is_ok() {
            return client_ip.to_string();
        }
        // 进程内稳定即可：桶只在进程内存中存活，无需跨重启一致。
        let mut hasher = DefaultHasher::new();
        client_ip.hash(&mut hasher);
        let h = hasher.finish();
        std::net::Ipv6Addr::new(
            0xfc00,
            0,
            0,
            0,
            (h >> 48) as u16,
            (h >> 32) as u16,
            (h >> 16) as u16,
            h as u16,
        )
        .to_string()
    }

    /// 组装 `BruteForceStrategy`：每实例一份进程内 DAO + 策略（与手写版实例级
    /// 内存桶等价）；DAO 外包 `BoundedDao` 容量阀补回键空间上界。
    #[cfg(feature = "garrison-auth")]
    fn build_brute_force_strategy() -> garrison::strategy::firewall::BruteForceStrategy {
        use garrison::strategy::firewall::{BruteForceConfig, BruteForceStrategy};

        let dao: Arc<dyn garrison::dao::GarrisonDao> =
            Arc::new(crate::server::auth::BoundedDao::new(
                crate::server::auth::InMemoryDao::new(),
                MAX_TRACKED_AUTH_FAILURE_KEYS,
            ));
        BruteForceStrategy::new(
            BruteForceConfig {
                max_attempts: AUTH_FAILURE_MAX_ATTEMPTS,
                window_seconds: AUTH_FAILURE_WINDOW_SECS,
                lock_seconds: AUTH_FAILURE_LOCK_SECS,
            },
            dao,
        )
    }

    /// 启用认证缓存；命中即跳过 DB + Argon2id 校验。
    ///
    /// TTL 与失效语义见 [`crate::server::auth::AuthCache`]。
    #[cfg(feature = "garrison-auth")]
    pub fn with_cache(mut self, cache: Arc<crate::server::auth::AuthCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// 认证是否启用（gRPC 侧据此决定放行/校验）。
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 设置可信代理 IP：仅直连 peer 在此列表时，信任 `X-Forwarded-For` /
    /// `X-Real-IP` 用于失败追踪；不可信 peer 一律按直连 IP 计数，防伪造头绕过。
    pub fn with_trusted_proxies(mut self, proxies: Vec<IpAddr>) -> Self {
        self.trusted_proxies = proxies;
        self
    }

    /// 认证失败速率检查：同一客户端 IP 在 5 分钟窗口内失败 ≥ 10 次则拒绝
    /// （返回 `false`）。
    ///
    /// gRPC 认证失败路径复用同一失败桶（HTTP 429 对应 gRPC
    /// `Code::ResourceExhausted`）；garrison-auth 构建委托
    /// `BruteForceStrategy::is_blocked`。
    #[cfg(feature = "garrison-auth")]
    pub(crate) async fn check_auth_failure_rate(&self, client_ip: &str) -> bool {
        use garrison::strategy::firewall::FirewallContext;

        let ctx = FirewallContext::new(Self::client_ip_to_context_ip(client_ip));
        match self.brute_force.is_blocked(&ctx).await {
            // is_blocked=true 表示已封禁 → 本方法返回 false（拒绝）。
            // DAO 故障时失败关闭：读不到封禁状态即拒绝，不得因取不到计数就
            // 放行爆破窗口；InMemoryDao 不会出错，换 Redis 后端 DAO 时该分支
            // 才可达（test_failure_rate_check_fails_closed）。
            Ok(blocked) => !blocked,
            Err(e) => {
                tracing::error!(
                    event = "auth_failure_rate_check_failed",
                    client_ip = %client_ip,
                    error = %e,
                    "{}",
                    t!("log.server.middleware.api_key_auth.failure_rate_check_failed")
                );
                false
            }
        }
    }

    /// 剪裁构建回退（无 garrison-auth）：进程内滑窗。
    #[cfg(not(feature = "garrison-auth"))]
    pub(crate) async fn check_auth_failure_rate(&self, client_ip: &str) -> bool {
        let now = Instant::now();
        let mut failures_map = self.auth_failures.write();
        let failures = failures_map.entry(client_ip.to_string()).or_default();

        // 移除 5 分钟前的记录
        failures.retain(|t| now.duration_since(*t) < Duration::from_secs(AUTH_FAILURE_WINDOW_SECS));

        // 清空空条目，避免长进程为每个曾出现的 IP 永久保留一个死 Vec
        // （轮换 IP 的攻击者可在失败窗口过期后仍撑爆内存）。
        if failures.is_empty() {
            failures_map.remove(client_ip);
            return true;
        }

        // 5 分钟内失败达到 10 次即阻止
        if failures.len() as u64 >= AUTH_FAILURE_MAX_FAILURES {
            tracing::warn!(
                client_ip = %client_ip,
                failure_count = failures.len(),
                "{}",
                t!("log.server.middleware.api_key_auth.too_many_auth_failures")
            );
            return false;
        }

        // 容量阀：先丢弃零失败条目，优先保留活跃失败 IP 而非新 IP。
        if failures_map.len() > MAX_TRACKED_AUTH_FAILURE_IPS {
            failures_map.retain(|_, v| !v.is_empty());
            if failures_map.len() > MAX_TRACKED_AUTH_FAILURE_IPS {
                // 仍超上限时按「每 IP 最早失败时间」淘汰最旧条目，替代 clear()：
                // 清空会抹掉活跃攻击者的失败计数，等于立即给攻击者全新 10 次预算。
                let excess = failures_map.len() - MAX_TRACKED_AUTH_FAILURE_IPS;
                let mut oldest_per_ip: Vec<(String, Instant)> = failures_map
                    .iter()
                    .map(|(ip, failures)| {
                        // 失败按时间 push，首元素即最旧；桶经上方 retain 后非空。
                        (ip.clone(), failures.first().copied().unwrap_or(now))
                    })
                    .collect();
                oldest_per_ip.sort_by_key(|(_, oldest)| *oldest);
                for (ip, _) in oldest_per_ip.into_iter().take(excess) {
                    failures_map.remove(&ip);
                }
            }
        }

        true
    }

    /// 记录一次认证失败（按客户端 IP 入桶）；gRPC 拒绝出口与 HTTP 中间件
    /// 共享同一失败桶（见 [`Self::check_auth_failure_rate`]）。
    ///
    /// garrison-auth 委托 `BruteForceStrategy::record_failure`（原子递增 + 窗口 TTL）。
    #[cfg(feature = "garrison-auth")]
    pub(crate) async fn record_auth_failure(&self, client_ip: &str) {
        use garrison::strategy::firewall::FirewallContext;

        let ctx = FirewallContext::new(Self::client_ip_to_context_ip(client_ip));
        if let Err(e) = self.brute_force.record_failure(&ctx).await {
            // 失败必须显性上报：静默丢计数等于给攻击者免费失败次数
            tracing::error!(
                event = "auth_failure_record_failed",
                client_ip = %client_ip,
                error = %e,
                "{}",
                t!("log.server.middleware.api_key_auth.failure_record_failed")
            );
        }
    }

    /// 剪裁构建回退（无 garrison-auth）：进程内滑窗。
    #[cfg(not(feature = "garrison-auth"))]
    pub(crate) async fn record_auth_failure(&self, client_ip: &str) {
        let now = Instant::now();
        let mut failures_map = self.auth_failures.write();
        let failures = failures_map.entry(client_ip.to_string()).or_default();
        failures.push(now);
    }

    fn too_many_requests_response(&self) -> Response {
        let response = axum::Json(serde_json::json!({
            "code": 429,
            "message": "Too many authentication attempts. Please try again later."
        }))
        .into_response();
        (StatusCode::TOO_MANY_REQUESTS, response).into_response()
    }

    fn get_client_ip(&self, req: &Request<Body>) -> Option<String> {
        // 委托共享的 trusted-proxy 感知实现；此前盲信 X-Forwarded-For，
        // 攻击者可伪造头部绕过 per-IP 认证失败限速。
        crate::server::middleware::utils::get_client_ip(req, &self.trusted_proxies)
    }

    pub async fn validate_key(&self, key_id: &str, key_secret: &str) -> Option<AuthenticatedKey> {
        #[cfg(feature = "garrison-auth")]
        if let Some(cache) = self.cache.as_ref() {
            if let Some(identity) = cache.get(key_id, key_secret).await {
                tracing::debug!(
                    event = "auth_cache_hit",
                    key_id_prefix = %key_id.chars().take(8).collect::<String>(),
                    "authentication served from cache"
                );
                // 缓存只存当代凭证命中的决策（宽限期命中不写缓存），恢复时一律非宽限期。
                return Some(AuthenticatedKey {
                    workspace_id: identity.workspace_id,
                    role: identity.role,
                    used_previous_credential: false,
                });
            }
        }

        let auth = self
            .repo
            .validate_api_key(key_id, key_secret)
            .await
            .ok()
            .flatten()?;

        // 上一代凭证只在宽限期内有效，而缓存只能表达相对 TTL：写入会在宽限期
        // 关闭后继续放行直到 TTL 到期，等于变相延长窗口。故不进缓存，
        // 并记显式告警供运维定位尚未完成轮换的调用方。
        if auth.used_previous_credential {
            tracing::warn!(
                event = "auth_grace_credential_used",
                key_id_prefix = %key_id.chars().take(8).collect::<String>(),
                "request authenticated with a previous-generation credential inside the rotation grace period"
            );
            return Some(auth);
        }

        #[cfg(feature = "garrison-auth")]
        if let Some(cache) = self.cache.as_ref() {
            // 缓存必须带 key 绝对过期时间，否则「TTL 未到但 key 已到期」会继续放行。
            let key_row = match self.repo.get_api_key_by_id(key_id).await {
                Ok(info) => info,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        key_id_prefix = %key_id.chars().take(8).collect::<String>(),
                        "cannot read key expiry; skipping auth cache write"
                    );
                    None
                }
            };
            if let Some(info) = key_row {
                cache
                    .put(
                        key_id,
                        key_secret,
                        &crate::server::auth::CachedIdentity {
                            workspace_id: auth.workspace_id,
                            role: auth.role.clone(),
                            key_expires_at: info
                                .expires_at
                                .map(|expires_at| expires_at.and_utc().timestamp()),
                        },
                    )
                    .await;
            }
        }

        Some(auth)
    }

    pub async fn auth_middleware(&self, mut req: Request<Body>, next: Next) -> Response {
        let start_time = Instant::now();
        let path = req.uri().path().to_string();

        let client_ip = self
            .get_client_ip(&req)
            .unwrap_or_else(|| "unknown".to_string());
        let user_agent = req
            .headers()
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        tracing::debug!(event = "auth_middleware", path = %path, client_ip = %client_ip, "{}", t!("log.server.middleware.api_key_auth.auth_middleware_called"));

        // 禁用认证时记录警告日志并设置默认扩展值；仍须留审计痕迹。
        if !self.enabled {
            tracing::warn!(
                event = "auth_disabled_request",
                path = %path,
                client_ip = %client_ip,
                user_agent = %user_agent,
                "{}",
                t!("log.server.middleware.api_key_auth.auth_disabled_request")
            );

            // 注入默认 workspace_id（None）与 Anonymous 角色。
            req.extensions_mut().insert(None::<uuid::Uuid>);
            // 禁用认证不得赋予 User 角色（有生成 ID 等业务权限）：Anonymous
            // 权限更低，只能访问公开端点，其余由 router::verify_user_role 拒绝。
            req.extensions_mut().insert(ApiKeyRole::Anonymous);

            // 无审计日志器句柄，只能留结构化日志（审计由 router 层中间件负责）。
            tokio::spawn(async move {
                tracing::info!(
                    event = "audit_auth_disabled",
                    path = %path,
                    client_ip = %client_ip,
                    "{}",
                    t!("log.server.middleware.api_key_auth.request_processed_without_auth")
                );
            });

            return next.run(req).await;
        }

        // 检查认证失败速率
        if !self.check_auth_failure_rate(&client_ip).await {
            return self.too_many_requests_response();
        }

        let auth_header = req.headers().get("authorization").cloned();

        // 解析改调全仓唯一实现（此前另有第二份手写解析且语义分歧，
        // 空凭证一处拒绝一处延后）；失败原因仍逐类打点。
        let parsed = auth_header
            .as_ref()
            .map(|header| {
                header
                    .to_str()
                    .map_err(|_| AuthHeaderError::InvalidEncoding)
                    .and_then(parse_authorization_header_detailed)
            })
            .transpose();

        let (key_id, key_secret) = match parsed {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                tracing::warn!(
                    event = "auth_failure",
                    reason = "missing_auth_header",
                    client_ip = %client_ip,
                    "{}",
                    t!("log.server.middleware.api_key_auth.missing_auth_header")
                );
                return self.unauthorized_response(&client_ip).await;
            }
            Err(err) => {
                let (reason, message) = match err {
                    AuthHeaderError::UnsupportedFormat => (
                        "unsupported_format",
                        t!("log.server.middleware.api_key_auth.unsupported_auth_format"),
                    ),
                    AuthHeaderError::Base64DecodeFailed => (
                        "base64_decode_failed",
                        t!("log.server.middleware.api_key_auth.base64_decode_failed"),
                    ),
                    AuthHeaderError::InvalidEncoding => (
                        "invalid_encoding",
                        t!("log.server.middleware.api_key_auth.invalid_base64_encoding"),
                    ),
                    AuthHeaderError::InvalidBasicFormat => (
                        "invalid_basic_format",
                        t!("log.server.middleware.api_key_auth.invalid_basic_format"),
                    ),
                    AuthHeaderError::InvalidApikeyFormat => (
                        "invalid_apikey_format",
                        t!("log.server.middleware.api_key_auth.invalid_apikey_format"),
                    ),
                    AuthHeaderError::EmptyCredentials => (
                        "empty_credentials",
                        t!("log.server.middleware.api_key_auth.empty_credentials"),
                    ),
                };
                tracing::warn!(
                    event = "auth_failure",
                    reason = %reason,
                    client_ip = %client_ip,
                    "{}",
                    message
                );
                return self.unauthorized_response(&client_ip).await;
            }
        };

        match self.validate_key(&key_id, &key_secret).await {
            Some(auth) => {
                req.extensions_mut().insert(auth.workspace_id);
                req.extensions_mut().insert(auth.role.clone());

                let duration = start_time.elapsed().as_millis() as u64;
                let key_id_prefix = key_id.chars().take(8).collect::<String>();
                tracing::info!(
                    event = "auth_success",
                    key_id_prefix = %key_id_prefix,
                    role = ?auth.role,
                    client_ip = %client_ip,
                    duration_ms = duration,
                    "{}",
                    t!("log.server.middleware.api_key_auth.authentication_successful")
                );

                return next.run(req).await;
            }
            None => {
                // 失败日志只带 key_id 前缀（脱敏）
                let key_id_prefix = key_id.chars().take(8).collect::<String>();
                tracing::warn!(
                    event = "auth_failure",
                    reason = "invalid_credentials",
                    key_id_prefix = %key_id_prefix,
                    client_ip = %client_ip,
                    "{}",
                    t!("log.server.middleware.api_key_auth.invalid_credentials")
                );
            }
        }

        // 未知路由与缺失认证统一返回 401，避免暴露端点是否存在。
        self.unauthorized_response(&client_ip).await
    }

    async fn unauthorized_response(&self, client_ip: &str) -> Response {
        self.record_auth_failure(client_ip).await;
        let response = axum::Json(serde_json::json!({
            "code": 401,
            "message": "Invalid or missing API key"
        }))
        .into_response();
        (StatusCode::UNAUTHORIZED, response).into_response()
    }
}

/// `Authorization` 头解析失败原因（机器可读）。
///
/// 审计日志按本枚举分支输出 `reason`，不合并成一个笼统的「格式错误」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthHeaderError {
    /// 前缀既不是 `Basic ` 也不是 `ApiKey `
    UnsupportedFormat,
    /// Basic 凭证不是合法 base64
    Base64DecodeFailed,
    /// base64 解出的字节流不是 UTF-8，或头部本身含非 UTF-8 字节
    InvalidEncoding,
    /// Basic 载荷缺少 `key_id:key_secret` 结构
    InvalidBasicFormat,
    /// ApiKey 值缺少 `key_id:key_secret` 结构
    InvalidApikeyFormat,
    /// 结构正确但 `key_id` 或 `key_secret` 为空
    EmptyCredentials,
}

/// 解析 `Authorization` 头，失败时给出可审计的具体原因。
///
/// 支持 `Basic base64(key_id:key_secret)` 与 `ApiKey key_id:key_secret`；
/// 全仓唯一实现，HTTP 中间件与 gRPC 入口共用。
pub fn parse_authorization_header_detailed(
    value: &str,
) -> Result<(String, String), AuthHeaderError> {
    if let Some(credentials) = value.strip_prefix("Basic ") {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(credentials)
            .map_err(|_| AuthHeaderError::Base64DecodeFailed)?;
        let cred_str = String::from_utf8(decoded).map_err(|_| AuthHeaderError::InvalidEncoding)?;
        let (key_id, key_secret) =
            split_pair(&cred_str).ok_or(AuthHeaderError::InvalidBasicFormat)?;
        require_non_empty(key_id, key_secret)
    } else if let Some(api_key) = value.strip_prefix("ApiKey ") {
        let (key_id, key_secret) =
            split_pair(api_key).ok_or(AuthHeaderError::InvalidApikeyFormat)?;
        require_non_empty(key_id, key_secret)
    } else {
        Err(AuthHeaderError::UnsupportedFormat)
    }
}

/// `key_id:key_secret` 结构切分（至多一段冒号）。
fn split_pair(raw: &str) -> Option<(&str, &str)> {
    let mut parts = raw.splitn(2, ':');
    match (parts.next(), parts.next()) {
        (Some(key_id), Some(key_secret)) => Some((key_id, key_secret)),
        _ => None,
    }
}

fn require_non_empty(key_id: &str, key_secret: &str) -> Result<(String, String), AuthHeaderError> {
    if key_id.is_empty() || key_secret.is_empty() {
        return Err(AuthHeaderError::EmptyCredentials);
    }
    Ok((key_id.to_string(), key_secret.to_string()))
}

/// Authorization 头解析的共享便捷版：任何问题统一返回 `None`。
/// 需要区分失败原因（写审计日志）用 [`parse_authorization_header_detailed`]。
pub fn parse_authorization_header(value: &str) -> Option<(String, String)> {
    parse_authorization_header_detailed(value).ok()
}

pub async fn admin_required_middleware(req: Request<Body>, next: Next) -> Response {
    if let Some(role) = req.extensions().get::<ApiKeyRole>() {
        tracing::debug!(event = "admin_check", role = ?role, "{}", t!("log.server.middleware.api_key_auth.checking_admin_role"));
        if *role == ApiKeyRole::Admin {
            return next.run(req).await;
        }
    } else {
        tracing::warn!(
            event = "admin_check",
            "{}",
            t!("log.server.middleware.api_key_auth.no_api_key_role_extension")
        );
    }

    let response = axum::Json(serde_json::json!({
        "code": 403,
        "message": "Admin access required"
    }))
    .into_response();
    (StatusCode::FORBIDDEN, response).into_response()
}

pub async fn auth_middleware_fn(
    State(auth): State<Arc<ApiKeyAuth>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    auth.auth_middleware(req, next).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::database::{
        ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole, ApiKeyWithSecret,
        CreateApiKeyRequest,
    };
    use crate::core::types::Result;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::Router;
    use sdforge::tower::ServiceExt;
    use sha2::Digest;
    use uuid::Uuid;

    #[derive(Clone)]
    struct MockApiKeyRepo {
        keys: std::collections::HashMap<String, (String, ApiKeyRole)>,
    }

    impl MockApiKeyRepo {
        fn hash_secret(secret: &str) -> String {
            let mut hasher = sha2::Sha256::default();
            hasher.update(secret);
            hex::encode(hasher.finalize())
        }
    }

    #[async_trait]
    impl ApiKeyRepository for MockApiKeyRepo {
        async fn create_api_key(&self, _request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret> {
            Ok(ApiKeyWithSecret {
                key: ApiKeyResponse {
                    id: Uuid::new_v4(),
                    key_id: "mock_key_id".to_string(),
                    key_prefix: "nino_".to_string(),
                    name: "Mock Key".to_string(),
                    description: None,
                    role: ApiKeyRole::User,
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
            key_id: &str,
            key_secret: &str,
        ) -> Result<Option<AuthenticatedKey>> {
            use subtle::ConstantTimeEq;
            if let Some((expected_secret, role)) = self.keys.get(key_id) {
                let incoming_hash = MockApiKeyRepo::hash_secret(key_secret);
                if expected_secret
                    .as_bytes()
                    .ct_eq(incoming_hash.as_bytes())
                    .into()
                {
                    // Admin keys have None workspace_id, user keys have Some(workspace_id)
                    let workspace_id = if *role == ApiKeyRole::Admin {
                        None
                    } else {
                        Some(uuid::Uuid::nil())
                    };
                    return Ok(Some(AuthenticatedKey {
                        workspace_id,
                        role: role.clone(),
                        used_previous_credential: false,
                    }));
                }
            }
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

        /// admin 守卫用：本 mock 只存 key_id→(hash, role)，无行主键也无 enabled，恒查不到行。
        async fn find_api_key_by_row_id(&self, _id: Uuid) -> Result<Option<ApiKeyInfo>> {
            Ok(None)
        }

        /// admin 守卫用：本 mock 不建模 key 行，与 count_api_keys 一致返回 0。
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

        async fn get_keys_older_than(&self, _age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn test_api_key_auth_with_mock_repo() {
        let mut mock_keys = std::collections::HashMap::new();
        // hash_secret 只哈希 secret，与真实校验逻辑一致
        mock_keys.insert(
            "test-key-id".to_string(),
            (MockApiKeyRepo::hash_secret("test-secret"), ApiKeyRole::User),
        );
        mock_keys.insert(
            "admin-key".to_string(),
            (
                MockApiKeyRepo::hash_secret("admin-secret"),
                ApiKeyRole::Admin,
            ),
        );

        let repo = MockApiKeyRepo { keys: mock_keys };
        let auth = ApiKeyAuth::new(Arc::new(repo), true);

        let result = auth.validate_key("test-key-id", "test-secret").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().role, ApiKeyRole::User);

        let result = auth.validate_key("admin-key", "admin-secret").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().role, ApiKeyRole::Admin);

        let result = auth.validate_key("test-key-id", "wrong-secret").await;
        assert!(result.is_none());

        let result = auth.validate_key("non-existent", "secret").await;
        assert!(result.is_none());
    }

    // ========== Helper functions for middleware tests ==========

    fn make_mock_repo() -> MockApiKeyRepo {
        let mut mock_keys = std::collections::HashMap::new();
        mock_keys.insert(
            "user-key".to_string(),
            (MockApiKeyRepo::hash_secret("user-secret"), ApiKeyRole::User),
        );
        mock_keys.insert(
            "admin-key".to_string(),
            (
                MockApiKeyRepo::hash_secret("admin-secret"),
                ApiKeyRole::Admin,
            ),
        );
        MockApiKeyRepo { keys: mock_keys }
    }

    fn build_test_router(auth: Arc<ApiKeyAuth>) -> Router {
        Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(from_fn_with_state(auth, auth_middleware_fn))
    }

    fn basic_auth_header(key_id: &str, key_secret: &str) -> String {
        let credentials = format!("{}:{}", key_id, key_secret);
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        format!("Basic {}", encoded)
    }

    fn api_key_header(key_id: &str, key_secret: &str) -> String {
        format!("ApiKey {}:{}", key_id, key_secret)
    }

    fn make_request(auth_header: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri("/test").method("GET");
        if let Some(value) = auth_header {
            builder = builder.header("authorization", value);
        }
        builder.body(Body::empty()).unwrap()
    }

    // ========== Constructor tests ==========

    #[test]
    fn test_api_key_auth_new_enabled() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, true);
        assert!(auth.enabled);
    }

    #[test]
    fn test_api_key_auth_new_disabled() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, false);
        assert!(!auth.enabled);
    }

    #[test]
    fn test_api_key_auth_with_trusted_proxies_does_not_panic() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let proxies = vec!["127.0.0.1".parse().unwrap()];
        let auth = ApiKeyAuth::new(repo, true).with_trusted_proxies(proxies);
        assert!(auth.enabled);
    }

    // ========== auth_middleware tests ==========

    #[tokio::test]
    async fn test_auth_middleware_disabled_calls_next() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, false));
        let router = build_test_router(auth);
        let resp = router.oneshot(make_request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_middleware_enabled_no_auth_header_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let resp = router.oneshot(make_request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_valid_user_calls_next() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("user-key", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_valid_admin_calls_next() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("admin-key", "admin-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_invalid_credentials_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("user-key", "wrong-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_invalid_base64_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let resp = router
            .oneshot(make_request(Some("Basic !!!not-base64!!!")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_no_colon_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let encoded = base64::engine::general_purpose::STANDARD.encode("nocolonstring");
        let header = format!("Basic {}", encoded);
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_empty_key_id_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_empty_key_secret_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("user-key", "");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_api_key_valid_calls_next() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = api_key_header("user-key", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_middleware_api_key_no_colon_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = "ApiKey nocolonstring".to_string();
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_unsupported_format_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let resp = router
            .oneshot(make_request(Some("Bearer some-token")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_too_many_failures_returns_429() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        // 10 次失败（5 分钟窗口）触发 429
        let bad_header = basic_auth_header("user-key", "wrong");
        for _ in 0..10 {
            let _ = router
                .clone()
                .oneshot(make_request(Some(&bad_header)))
                .await
                .unwrap();
        }
        // 第 11 次请求应为 429
        let resp = router
            .oneshot(make_request(Some(&bad_header)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn test_auth_middleware_empty_authorization_value_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let resp = router.oneshot(make_request(Some(""))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ========== admin_required_middleware tests ==========

    fn build_admin_router() -> Router {
        // 仅挂 admin_required；角色 extension 由测试手动注入以隔离 admin 检查。
        Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(admin_required_middleware))
    }

    fn make_request_with_role(role: ApiKeyRole) -> Request<Body> {
        let mut builder = Request::builder().uri("/test").method("GET");
        builder = builder.extension(role);
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn test_admin_required_admin_role_calls_next() {
        let router = build_admin_router();
        let resp = router
            .oneshot(make_request_with_role(ApiKeyRole::Admin))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_admin_required_user_role_returns_403() {
        let router = build_admin_router();
        let resp = router
            .oneshot(make_request_with_role(ApiKeyRole::User))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_admin_required_no_role_extension_returns_403() {
        let router = build_admin_router();
        let resp = router.oneshot(make_request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_admin_required_anonymous_role_returns_403() {
        let router = build_admin_router();
        let resp = router
            .oneshot(make_request_with_role(ApiKeyRole::Anonymous))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ========== Role extension injection tests ==========

    fn build_role_check_router(auth: Arc<ApiKeyAuth>) -> Router {
        // 路由回显注入的 role，便于断言 auth_middleware 注入了哪个角色。
        Router::new()
            .route(
                "/test",
                get(|request: Request<Body>| async move {
                    if let Some(role) = request.extensions().get::<ApiKeyRole>() {
                        format!("{:?}", role)
                    } else {
                        "no-role".to_string()
                    }
                }),
            )
            .layer(from_fn_with_state(auth, auth_middleware_fn))
    }

    async fn read_body_to_string(body: Body) -> String {
        // 用 axum 内置 `to_bytes`，避免引入 http_body_util 开发依赖。
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .expect("failed to read response body");
        String::from_utf8(bytes.to_vec()).expect("response body is not valid UTF-8")
    }

    #[tokio::test]
    async fn test_auth_disabled_injects_anonymous_role() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, false));
        let router = build_role_check_router(auth);
        let resp = router.oneshot(make_request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_to_string(resp.into_body()).await;
        assert_eq!(body, "Anonymous");
    }

    #[tokio::test]
    async fn test_auth_valid_basic_injects_user_role() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_role_check_router(auth);
        let header = basic_auth_header("user-key", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_to_string(resp.into_body()).await;
        assert_eq!(body, "User");
    }

    #[tokio::test]
    async fn test_auth_valid_basic_admin_injects_admin_role() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_role_check_router(auth);
        let header = basic_auth_header("admin-key", "admin-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_to_string(resp.into_body()).await;
        assert_eq!(body, "Admin");
    }

    #[tokio::test]
    async fn test_auth_valid_api_key_injects_user_role() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_role_check_router(auth);
        let header = api_key_header("user-key", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_to_string(resp.into_body()).await;
        assert_eq!(body, "User");
    }

    // ========== validate_key tests ==========

    #[tokio::test]
    async fn test_validate_key_empty_key_id_returns_none() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, true);
        let result = auth.validate_key("", "user-secret").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_validate_key_empty_key_secret_returns_none() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, true);
        let result = auth.validate_key("user-key", "").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_validate_key_admin_returns_none_workspace_id() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, true);
        let result = auth.validate_key("admin-key", "admin-secret").await;
        assert!(result.is_some());
        let auth = result.unwrap();
        assert_eq!(auth.role, ApiKeyRole::Admin);
        // Admin key 全局有效（workspace_id = None）
        assert!(auth.workspace_id.is_none());
    }

    #[tokio::test]
    async fn test_validate_key_user_returns_some_workspace_id() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, true);
        let result = auth.validate_key("user-key", "user-secret").await;
        assert!(result.is_some());
        let auth = result.unwrap();
        assert_eq!(auth.role, ApiKeyRole::User);
        // User key 绑定 workspace（mock 返回 Some(Uuid::nil())）
        assert!(auth.workspace_id.is_some());
        assert_eq!(auth.workspace_id.unwrap(), Uuid::nil());
    }

    // ===== 解析唯一实现 =====

    #[test]
    fn parse_detailed_accepts_both_schemes_and_classifies_every_failure() {
        let ok_basic = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("k1:s1")
        );
        assert_eq!(
            parse_authorization_header_detailed(&ok_basic).ok(),
            Some(("k1".to_string(), "s1".to_string()))
        );
        assert_eq!(
            parse_authorization_header_detailed("ApiKey k2:s3").ok(),
            Some(("k2".to_string(), "s3".to_string()))
        );
        // secret 内含冒号时只按第一个冒号切分，剩余部分归 secret
        assert_eq!(
            parse_authorization_header_detailed("ApiKey k:s:t").ok(),
            Some(("k".to_string(), "s:t".to_string()))
        );

        assert_eq!(
            parse_authorization_header_detailed("Bearer xyz"),
            Err(AuthHeaderError::UnsupportedFormat)
        );
        assert_eq!(
            parse_authorization_header_detailed("Basic !!!not-base64"),
            Err(AuthHeaderError::Base64DecodeFailed)
        );
        // base64 合法但解出的字节不是 UTF-8
        let bad_utf8 = base64::engine::general_purpose::STANDARD.encode([0xff_u8, 0xfe_u8]);
        assert_eq!(
            parse_authorization_header_detailed(&format!("Basic {bad_utf8}")),
            Err(AuthHeaderError::InvalidEncoding)
        );
        // "hello"：无冒号 ⇒ Basic 结构错
        let no_colon = base64::engine::general_purpose::STANDARD.encode(b"hello");
        assert_eq!(
            parse_authorization_header_detailed(&format!("Basic {no_colon}")),
            Err(AuthHeaderError::InvalidBasicFormat)
        );
        assert_eq!(
            parse_authorization_header_detailed("ApiKey noseparator"),
            Err(AuthHeaderError::InvalidApikeyFormat)
        );
        assert_eq!(
            parse_authorization_header_detailed("ApiKey :secret"),
            Err(AuthHeaderError::EmptyCredentials)
        );
        assert_eq!(
            parse_authorization_header_detailed("ApiKey keyid:"),
            Err(AuthHeaderError::EmptyCredentials)
        );
    }

    #[test]
    fn parse_wrapper_never_diverges_from_the_single_implementation() {
        // 此前 HTTP 中间件有第二份内联解析且空凭证判定不一致；包装必须逐例同结论。
        for value in [
            "ApiKey k:s",
            "ApiKey :s",
            "ApiKey k:",
            "ApiKey nope",
            "Bearer nope",
            "Basic !!!",
        ] {
            assert_eq!(
                parse_authorization_header(value),
                parse_authorization_header_detailed(value).ok(),
                "包装与唯一实现在 {value:?} 上结论不一致"
            );
        }
    }

    /// 活跃攻击者存活语义：灌入 10,002 个其他 IP（未达 100k 容量阀）不得重置
    /// 活跃攻击者的失败预算；有界性与淘汰极性由 memory_dao.rs 测试直接锚定。
    #[tokio::test]
    async fn test_active_attacker_counter_survives_many_other_ips() {
        let auth = ApiKeyAuth::new(Arc::new(make_mock_repo()), true);

        // 攻击者先失败 10 次：达阈值并被封禁。
        for _ in 0..10 {
            auth.record_auth_failure("203.0.113.7").await;
        }
        assert!(!auth.check_auth_failure_rate("203.0.113.7").await);

        // 灌注 10_002 个其他 IP（未达容量阀），验证不重置活跃攻击者的失败预算。
        for i in 0..10_002u32 {
            auth.record_auth_failure(&format!("10.9.{}.{}", i / 256, i % 256))
                .await;
        }

        // 关键语义回归：活跃攻击者的失败预算未被重置 → 仍被拒绝。
        assert!(
            !auth.check_auth_failure_rate("203.0.113.7").await,
            "不得因其他 IP 的灌注重置活跃攻击者的失败预算"
        );

        // 失败次数未达阈值的其他 IP 不受影响（每条目各计各的）。
        assert!(auth.check_auth_failure_rate("10.9.0.1").await);
    }

    /// H1 回归锚点：DAO 故障时 `check_auth_failure_rate` 必须失败关闭（返回 false），
    /// 与 i18n 文案「按失败关闭拒绝」一致；此前 Err 分支误返 true（放行）。
    #[tokio::test]
    #[cfg(feature = "garrison-auth")]
    async fn test_failure_rate_check_fails_closed_when_dao_errors() {
        use garrison::strategy::firewall::{BruteForceConfig, BruteForceStrategy};

        /// 全部写读路径恒失败的 DAO 桩：模拟 Redis 后端不可达。
        struct ErrDao;

        #[async_trait]
        impl garrison::dao::GarrisonDao for ErrDao {
            async fn get(&self, _key: &str) -> garrison::error::GarrisonResult<Option<String>> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
            async fn set(
                &self,
                _key: &str,
                _value: &str,
                _ttl_seconds: u64,
            ) -> garrison::error::GarrisonResult<()> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
            async fn update(
                &self,
                _key: &str,
                _value: &str,
            ) -> garrison::error::GarrisonResult<()> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
            async fn expire(
                &self,
                _key: &str,
                _seconds: u64,
            ) -> garrison::error::GarrisonResult<()> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
            async fn delete(&self, _key: &str) -> garrison::error::GarrisonResult<()> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
            async fn set_if_absent(
                &self,
                _key: &str,
                _value: &str,
                _ttl_seconds: u64,
            ) -> garrison::error::GarrisonResult<bool> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
            async fn rename(
                &self,
                _old_key: &str,
                _new_key: &str,
            ) -> garrison::error::GarrisonResult<()> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
            async fn get_and_delete(
                &self,
                _key: &str,
            ) -> garrison::error::GarrisonResult<Option<String>> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
            async fn incr(
                &self,
                _key: &str,
                _ttl_seconds: u64,
            ) -> garrison::error::GarrisonResult<u64> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
            async fn decr(&self, _key: &str) -> garrison::error::GarrisonResult<u64> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
            async fn compare_and_swap(
                &self,
                _key: &str,
                _expected: Option<&str>,
                _new_value: &str,
                _ttl_seconds: u64,
            ) -> garrison::error::GarrisonResult<bool> {
                Err(garrison::error::GarrisonError::Dao("probe".into()))
            }
        }

        let strategy = BruteForceStrategy::new(
            BruteForceConfig {
                max_attempts: AUTH_FAILURE_MAX_ATTEMPTS,
                window_seconds: AUTH_FAILURE_WINDOW_SECS,
                lock_seconds: AUTH_FAILURE_LOCK_SECS,
            },
            Arc::new(ErrDao),
        );
        let auth = ApiKeyAuth {
            repo: Arc::new(make_mock_repo()),
            enabled: true,
            trusted_proxies: Vec::new(),
            brute_force: Arc::new(strategy),
            cache: None,
        };
        assert!(
            !auth.check_auth_failure_rate("203.0.113.7").await,
            "DAO 故障必须失败关闭：读不到封禁状态即拒绝"
        );
    }

    #[tokio::test]
    async fn test_validate_key_previous_credential_skips_cache() {
        use crate::server::handlers::mock_tests::MockApiKeyRepository;

        let mut mock = MockApiKeyRepository::new();
        mock.expect_validate_api_key().returning(|_, _| {
            Ok(Some(AuthenticatedKey {
                workspace_id: None,
                role: ApiKeyRole::User,
                used_previous_credential: true,
            }))
        });
        let auth = ApiKeyAuth::new(Arc::new(mock), true);
        let got = auth
            .validate_key("k", "old-secret")
            .await
            .expect("grace credential must authenticate");
        assert!(got.used_previous_credential);
    }

    #[cfg(feature = "garrison-auth")]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_validate_key_cache_hit_skips_repository() {
        use crate::server::auth::AuthCache;
        use crate::server::handlers::mock_tests::MockApiKeyRepository;

        fn info() -> crate::core::database::ApiKeyInfo {
            let now = chrono::Utc::now().naive_utc();
            crate::core::database::ApiKeyInfo {
                id: Uuid::new_v4(),
                key_id: "k".to_string(),
                key_prefix: "k".to_string(),
                role: ApiKeyRole::User,
                workspace_id: None,
                name: "test".to_string(),
                description: None,
                rate_limit: 100,
                enabled: true,
                expires_at: None,
                last_used_at: None,
                created_at: now,
            }
        }

        let mut mock = MockApiKeyRepository::new();
        mock.expect_validate_api_key().times(1).returning(|_, _| {
            Ok(Some(AuthenticatedKey {
                workspace_id: None,
                role: ApiKeyRole::User,
                used_previous_credential: false,
            }))
        });
        mock.expect_get_api_key_by_id()
            .returning(|_| Ok(Some(info())));
        let auth =
            ApiKeyAuth::new(Arc::new(mock), true).with_cache(Arc::new(AuthCache::new(300).await));

        let first = auth.validate_key("k", "s").await;
        assert!(first.is_some());
        // 第二次必须走缓存（mock 限定 validate 只调一次，否则 panic）。
        let second = auth.validate_key("k", "s").await;
        assert!(second.is_some());
        assert!(!second.unwrap().used_previous_credential);
    }

    #[tokio::test]
    async fn test_middleware_disabled_auth_passthrough() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, false));
        let router = build_test_router(auth);
        let resp = router.oneshot(make_request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_middleware_malformed_headers_rejected() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        for header in [
            "Bearer sometoken",
            "Basic !!!",
            "ApiKey no-colon-here",
            "Basic Og==",
        ] {
            let auth = Arc::new(ApiKeyAuth::new(repo.clone(), true));
            let router = build_test_router(auth);
            let resp = router.oneshot(make_request(Some(header))).await.unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::UNAUTHORIZED,
                "{header:?} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn test_middleware_valid_key_succeeds() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("user-key", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_middleware_rate_limits_repeated_failures() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let bad = basic_auth_header("user-key", "wrong-secret");
        let mut last_status = StatusCode::UNAUTHORIZED;
        for _ in 0..12 {
            let router = build_test_router(auth.clone());
            let resp = router.oneshot(make_request(Some(&bad))).await.unwrap();
            last_status = resp.status();
        }
        assert_eq!(last_status, StatusCode::TOO_MANY_REQUESTS);
    }

    /// 5 分钟窗口 10 次阈值：`BruteForceStrategy` 承接后的语义钉桩。
    #[tokio::test]
    async fn test_check_auth_failure_rate_blocks_after_threshold() {
        let auth = ApiKeyAuth::new(Arc::new(make_mock_repo()), true);

        // 无失败记录的 IP 直接放行。
        assert!(auth.check_auth_failure_rate("192.0.2.50").await);

        // 恰好 10 次失败后命中阈值分支：放行翻转为拒绝。
        for _ in 0..10 {
            auth.record_auth_failure("192.0.2.50").await;
        }
        assert!(!auth.check_auth_failure_rate("192.0.2.50").await);

        // 阈值按 IP 隔离，其他 IP 不受影响。
        assert!(auth.check_auth_failure_rate("192.0.2.51").await);
    }

    /// HTTP 429 与 gRPC `ResourceExhausted` 共享同一失败桶：两条传输线共用
    /// 同一 `ApiKeyAuth` 实例与策略，不得出现第二份计数器。这里以 gRPC 调用
    /// 配对灌满桶，再用 HTTP 中间件断言同一 IP 已被拒（反向亦然）。
    #[tokio::test]
    async fn test_http_and_grpc_paths_share_one_failure_bucket() {
        let auth = Arc::new(ApiKeyAuth::new(Arc::new(make_mock_repo()), true));
        let router = build_test_router(Arc::clone(&auth));

        // gRPC 侧入桶 10 次；client 标识取 HTTP 中间件在本测试请求下的同一取值：
        // 请求无 ConnectInfo，`get_client_ip` 兜底为 "unknown"。
        for _ in 0..10 {
            auth.record_auth_failure("unknown").await;
        }
        // HTTP 中间件必须看到同一桶已满 → 429。
        let resp = router
            .oneshot(make_request(Some(&basic_auth_header(
                "user-key",
                "user-secret",
            ))))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "HTTP 与 gRPC 必须共享同一失败桶"
        );

        // 反向：HTTP 侧入桶后，gRPC 入口查询 `check_auth_failure_rate` 必须看到拒绝。
        let auth2 = Arc::new(ApiKeyAuth::new(Arc::new(make_mock_repo()), true));
        let router2 = build_test_router(Arc::clone(&auth2));
        let bad = basic_auth_header("user-key", "wrong");
        for _ in 0..10 {
            let _ = router2
                .clone()
                .oneshot(make_request(Some(&bad)))
                .await
                .unwrap();
        }
        assert!(
            !auth2.check_auth_failure_rate("unknown").await,
            "HTTP 侧灌满的桶必须对 gRPC 查询入口生效"
        );
    }

    /// 宽限期内的旧凭证：认证放行但跳过缓存写入（缓存只表达相对 TTL，
    /// 写入会变相延长宽限窗口），且显式留痕。
    #[cfg(feature = "garrison-auth")]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_grace_credential_authenticates_without_cache_write() {
        use crate::server::auth::AuthCache;
        use crate::server::handlers::mock_tests::MockApiKeyRepository;

        let mut mock = MockApiKeyRepository::new();
        mock.expect_validate_api_key().times(1).returning(|_, _| {
            Ok(Some(AuthenticatedKey {
                workspace_id: None,
                role: ApiKeyRole::User,
                used_previous_credential: true,
            }))
        });
        // 宽限路径必须在读 key 行之前返回；若退化去写缓存，times(0) 会让 mock panic。
        mock.expect_get_api_key_by_id().times(0);
        let auth =
            ApiKeyAuth::new(Arc::new(mock), true).with_cache(Arc::new(AuthCache::new(300).await));

        let result = auth.validate_key("k", "old-secret").await;
        let result = result.expect("宽限凭证必须放行");
        assert!(
            result.used_previous_credential,
            "放行结果必须携带宽限语义标记"
        );
    }

    /// key 行读取失败（异常或删除中）：宁可不缓存，也不能写入缺失过期时间的
    /// 条目；回源校验本身照常放行。
    #[cfg(feature = "garrison-auth")]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_cache_write_skipped_when_key_row_lookup_fails() {
        use crate::core::CoreError;
        use crate::server::auth::AuthCache;
        use crate::server::handlers::mock_tests::MockApiKeyRepository;

        let mut mock = MockApiKeyRepository::new();
        mock.expect_validate_api_key().times(1).returning(|_, _| {
            Ok(Some(AuthenticatedKey {
                workspace_id: None,
                role: ApiKeyRole::User,
                used_previous_credential: false,
            }))
        });
        mock.expect_get_api_key_by_id()
            .returning(|_| Err(CoreError::InternalError("row gone".into())));
        let auth =
            ApiKeyAuth::new(Arc::new(mock), true).with_cache(Arc::new(AuthCache::new(300).await));

        let result = auth.validate_key("k", "s").await;
        assert!(result.is_some(), "行读取失败只影响缓存写入，不影响放行");
    }
}
