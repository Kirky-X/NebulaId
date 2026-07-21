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

use crate::core::algorithm::{AuditEvent as CoreAuditEvent, AuditLogger as CoreAuditLoggerTrait};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::info;

// M4 修复：复用 core 层的 AuditEventType 和 AuditResult，
// 消除 server/core 重复定义。pub use 使其成为本模块公共 API。
pub use crate::core::algorithm::{AuditEventType, AuditResult};

/// 生成审计事件 ID（M1 修复）。
///
/// 格式：`(unix_millis << 20) | (counter & 0xFFFFF)`
/// - 高 44 位：Unix 毫秒时间戳，保证进程重启后 ID 单调递增、不冲突
/// - 低 20 位：进程内计数器（最多 1M/ms，远超任何审计吞吐）
///
/// 这避免了原 `static COUNTER: AtomicU64` 在进程重启后从 0 开始导致 ID 冲突的问题。
fn next_audit_event_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unix_ms = Utc::now().timestamp_millis() as u64;
    let c = COUNTER.fetch_add(1, Ordering::SeqCst) & 0xFFFFF;
    (unix_ms << 20) | c
}

// M4 修复：删除本地 AuditEventType 和 AuditResult 定义，
// 改用 `crate::core::algorithm::{AuditEventType, AuditResult}`。
// 这消除了 server/core 重复定义，未来新增事件类型只需修改 core 层一处。

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: u64,
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub workspace_id: Option<String>,
    pub user_id: Option<String>,
    pub action: String,
    pub resource: String,
    pub result: AuditResult,
    pub details: Option<serde_json::Value>,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
    pub duration_ms: u64,
    pub error_message: Option<String>,
}

impl AuditEvent {
    pub fn new(
        event_type: AuditEventType,
        workspace_id: Option<String>,
        action: String,
        resource: String,
        result: AuditResult,
    ) -> Self {
        Self {
            id: next_audit_event_id(),
            timestamp: Utc::now(),
            event_type,
            workspace_id,
            user_id: None,
            action,
            resource,
            result,
            details: None,
            client_ip: None,
            user_agent: None,
            duration_ms: 0,
            error_message: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_client_ip(mut self, ip: String) -> Self {
        self.client_ip = Some(ip);
        self
    }

    pub fn with_user_agent(mut self, ua: String) -> Self {
        self.user_agent = Some(ua);
        self
    }

    pub fn with_user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    pub fn with_error(mut self, error: String) -> Self {
        self.error_message = Some(error);
        self
    }

    /// LOW-3 修复 + SEC-LOW-001 修复：对 IP 地址进行部分遮蔽。
    ///
    /// 使用 `std::net::IpAddr` 解析而非字符串操作，避免 IPv4-mapped IPv6
    /// 等 boundary case 的脱敏不一致。原 `rfind('.')` / `rfind(':')` 方案
    /// 对 `::1` 返回 `:x`（不正确）、对 `2001:db8::1` 返回 `2001:db8:x`
    /// （与注释承诺的 `2001:db8::x` 不符）。
    fn redact_ip(ip: &str) -> String {
        match ip.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(v4)) => {
                let octets = v4.octets();
                format!("{}.{}.{}.x", octets[0], octets[1], octets[2])
            }
            Ok(std::net::IpAddr::V6(v6)) => {
                // 保留前 4 段，后面 4 段用 x 替换，避免泄露完整地址。
                let segs = v6.segments();
                format!(
                    "{:x}:{:x}:{:x}:{:x}:x:x:x:x",
                    segs[0], segs[1], segs[2], segs[3]
                )
            }
            Err(_) => {
                // 非 IP 格式（可能是 "unknown" 或其他占位符）：保留前 8 字符。
                let prefix = ip.chars().take(8).collect::<String>();
                if ip.len() > 8 {
                    format!("{}...redacted", prefix)
                } else {
                    prefix
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct AuditLogger {
    events: Arc<Mutex<VecDeque<AuditEvent>>>,
    max_events: usize,
    total_logged: Arc<AtomicU64>,
    total_errors: Arc<AtomicU64>,
    /// 文件写入 channel：log 方法只发送事件（非阻塞），独立 task 消费并写文件。
    /// 解决 H7：避免在 Mutex 持有期间执行文件 I/O。
    file_tx: Option<mpsc::UnboundedSender<AuditEvent>>,
    /// 持有 writer task 的 JoinHandle，保持 task 存活。
    /// Arc 包装以便 Clone 时共享；JoinHandle drop 不会 cancel task（tokio 默认行为），
    /// task 会在所有 sender drop 后自然退出。
    _writer_task: Option<Arc<JoinHandle<()>>>,
}

impl AuditLogger {
    pub fn new(max_events: usize) -> Self {
        Self {
            events: Arc::new(Mutex::new(VecDeque::with_capacity(max_events + 1))),
            max_events,
            total_logged: Arc::new(AtomicU64::new(0)),
            total_errors: Arc::new(AtomicU64::new(0)),
            file_tx: None,
            _writer_task: None,
        }
    }

    /// 创建支持文件持久化的审计日志记录器。
    ///
    /// 启动一个独立的后台 task 消费事件并写文件，`log` 方法只通过 channel
    /// 发送事件（非阻塞），避免在 Mutex 持有期间执行文件 I/O（H7 修复）。
    ///
    /// 必须在 tokio runtime 上下文中调用。
    ///
    /// LOW-2 修复（CWE-22 路径遍历）：验证 `log_file_path` 不包含 `..`
    /// 组件，防止攻击者通过配置注入如 `../../etc/passwd` 路径覆盖系统文件。
    /// 空路径也被拒绝。
    pub async fn with_file_logging(max_events: usize, log_file_path: String) -> Self {
        // LOW-2 修复：路径安全性验证
        if let Err(e) = Self::validate_log_path(&log_file_path) {
            tracing::error!(
                event = "audit_log_path_invalid",
                path = %log_file_path,
                error = %e,
                "{}",
                t!("log.server.audit.logger.path_invalid", error = e)
            );
            // 回退到无文件持久化的内存记录器，避免因路径无效导致启动失败
            return Self::new(max_events);
        }

        let (tx, mut rx) = mpsc::unbounded_channel::<AuditEvent>();
        let total_errors = Arc::new(AtomicU64::new(0));
        let errors_clone = total_errors.clone();
        let path_clone = log_file_path.clone();

        let handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(e) = Self::write_event_to_file(&event, &path_clone).await {
                    errors_clone.fetch_add(1, Ordering::SeqCst);
                    tracing::error!(
                        "{}",
                        t!("log.server.audit.logger.persist_failed", error = e)
                    );
                }
            }
        });

        Self {
            events: Arc::new(Mutex::new(VecDeque::with_capacity(max_events + 1))),
            max_events,
            total_logged: Arc::new(AtomicU64::new(0)),
            total_errors,
            file_tx: Some(tx),
            _writer_task: Some(Arc::new(handle)),
        }
    }

    /// LOW-2 修复（CWE-22）：验证审计日志路径安全性。
    ///
    /// 拒绝：
    /// - 空路径
    /// - 包含 `..` 组件的路径（路径遍历攻击）
    ///
    /// 允许：
    /// - 绝对路径（如 `/var/log/nebulaid/audit.log`）
    /// - 相对路径（如 `logs/audit.log`）
    fn validate_log_path(path: &str) -> std::io::Result<()> {
        use std::path::Component;

        if path.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "audit log path is empty",
            ));
        }

        let p = std::path::Path::new(path);
        for comp in p.components() {
            if matches!(comp, Component::ParentDir) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "audit log path contains '..' component (path traversal risk): {}",
                        path
                    ),
                ));
            }
        }

        Ok(())
    }

    pub async fn log(&self, event: AuditEvent) {
        // 锁内只做内存操作（push/pop），快速释放锁
        {
            let mut events = self.events.lock().await;
            // M14 修复：VecDeque 满时丢弃最旧事件。如果未配置文件持久化，
            // 丢弃意味着审计事件永久丢失（违反 SOC2/GDPR 合规）。
            // 此处记录 warning 提示运维人员配置 `audit_log_path`。
            if events.len() >= self.max_events {
                let dropped_count = events.len() - self.max_events + 1;
                for _ in 0..dropped_count {
                    if let Some(dropped) = events.pop_front() {
                        if self.file_tx.is_none() {
                            // 未配置文件持久化：丢弃即永久丢失
                            tracing::warn!(
                                event_id = dropped.id,
                                event_type = ?dropped.event_type,
                                "{}",
                                t!(
                                    "log.server.audit.logger.event_dropped_no_persistence",
                                    max_events = self.max_events
                                )
                            );
                            self.total_errors.fetch_add(1, Ordering::SeqCst);
                        }
                        // 已配置 file_tx 的事件已被异步写入文件，内存丢弃可接受
                    }
                }
            }
            events.push_back(event.clone());
            self.total_logged.fetch_add(1, Ordering::SeqCst);
        }

        // 锁外异步发送到文件 writer channel（非阻塞）
        if let Some(ref tx) = self.file_tx {
            if tx.send(event.clone()).is_err() {
                // channel 关闭（writer task panic 或退出）
                self.total_errors.fetch_add(1, Ordering::SeqCst);
                tracing::error!(
                    "{}",
                    t!(
                        "log.server.audit.logger.persist_failed",
                        error = "file writer channel closed"
                    )
                );
            }
        }

        info!(
            event_id = event.id,
            event_type = ?event.event_type,
            workspace = ?event.workspace_id,
            action = event.action,
            resource = event.resource,
            result = ?event.result,
            "{}",
            t!("log.server.audit.logger.audit_event_recorded")
        );
    }

    /// 将单个审计事件写入文件（由 writer task 调用，不在 Mutex 持有期间执行）。
    ///
    /// LOW-3 修复（CWE-532）：写入文件前对 PII 字段脱敏。
    /// - `client_ip`：保留前 3 段（IPv4）或前 4 段（IPv6），末段用 `x` 替换
    /// - `user_agent`：替换为固定字符串 `UA(redacted)`，避免记录完整 UA
    /// - `user_id`：保留（API key 标识符，非个人身份信息）
    ///
    /// PERF-M1 修复：原实现先 `redact_for_persistence()` 创建完整 AuditEvent
    /// 深拷贝（6-7 次 String clone + 1 次 Value 深拷贝），再 `to_string`。
    /// 现直接构建 `serde_json::Value`，仅对需要脱敏的 2 个字段做转换，
    /// 其余字段通过 `&` 引用序列化（serde 自动处理），避免深拷贝。
    /// 10k QPS 场景下减少约 1-2 万次/秒的 String 堆分配（注：`json!` 宏
    /// 仍会克隆 `redacted_client_ip` / `redacted_user_agent`，所以实际
    /// 节省的是其余字段的深拷贝，量级为 1-2 万次/秒）。
    async fn write_event_to_file(event: &AuditEvent, path: &str) -> std::io::Result<()> {
        use tokio::fs::OpenOptions;
        use tokio::io::AsyncWriteExt;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;

        // 仅对需要脱敏的 client_ip / user_agent 做转换，其余字段引用序列化。
        let redacted_client_ip = event.client_ip.as_deref().map(AuditEvent::redact_ip);
        let redacted_user_agent = event
            .user_agent
            .as_deref()
            .map(|_| "UA(redacted)".to_string());

        let log_line = serde_json::json!({
            "id": event.id,
            "timestamp": event.timestamp,
            "event_type": event.event_type,
            "workspace_id": event.workspace_id,
            "user_id": event.user_id,
            "action": event.action,
            "resource": event.resource,
            "result": event.result,
            "details": event.details,
            "client_ip": redacted_client_ip,
            "user_agent": redacted_user_agent,
            "duration_ms": event.duration_ms,
            "error_message": event.error_message,
        });

        let mut buf = Vec::with_capacity(512);
        serde_json::to_writer(&mut buf, &log_line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        buf.push(b'\n');
        file.write_all(&buf).await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn log_id_generation(
        &self,
        workspace_id: String,
        biz_tag: String,
        id: String,
        algorithm: String,
        client_ip: Option<String>,
        duration_ms: u64,
        success: bool,
        error_message: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            Some(workspace_id),
            "generate_id".to_string(),
            format!("biz_tag:{}", biz_tag),
            if success {
                AuditResult::Success
            } else {
                AuditResult::Failure
            },
        )
        .with_details(serde_json::json!({
            "generated_id": id,
            "algorithm": algorithm
        }))
        .with_client_ip(client_ip.unwrap_or_default())
        .with_duration(duration_ms);

        let event = if let Some(err) = error_message {
            event.with_error(err)
        } else {
            event
        };

        self.log(event).await;
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn log_batch_generation(
        &self,
        workspace_id: String,
        biz_tag: String,
        size: usize,
        client_ip: Option<String>,
        duration_ms: u64,
        success: bool,
        error_message: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::BatchGeneration,
            Some(workspace_id),
            "batch_generate_ids".to_string(),
            format!("biz_tag:{} size:{}", biz_tag, size),
            if success {
                AuditResult::Success
            } else {
                AuditResult::Failure
            },
        )
        .with_details(serde_json::json!({
            "batch_size": size,
            "biz_tag": biz_tag
        }))
        .with_client_ip(client_ip.unwrap_or_default())
        .with_duration(duration_ms);

        let event = if let Some(err) = error_message {
            event.with_error(err)
        } else {
            event
        };

        self.log(event).await;
    }

    pub async fn log_auth_event(
        &self,
        workspace_id: Option<String>,
        action: String,
        success: bool,
        client_ip: Option<String>,
        error_message: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::Authentication,
            workspace_id,
            action,
            "authentication".to_string(),
            if success {
                AuditResult::Success
            } else {
                AuditResult::Failure
            },
        )
        .with_client_ip(client_ip.unwrap_or_default());

        let event = if let Some(err) = error_message {
            event.with_error(err)
        } else {
            event
        };

        self.log(event).await;
    }

    pub async fn log_config_change(
        &self,
        workspace_id: Option<String>,
        action: String,
        config_type: String,
        details: serde_json::Value,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ConfigChange,
            workspace_id,
            action,
            format!("config:{}", config_type),
            AuditResult::Success,
        )
        .with_details(details);

        self.log(event).await;
    }

    pub async fn log_degradation_event(
        &self,
        workspace_id: Option<String>,
        action: String,
        algorithm_type: String,
        previous_state: String,
        current_state: String,
        details: serde_json::Value,
    ) {
        let event = AuditEvent::new(
            AuditEventType::DegradationEvent,
            workspace_id,
            action,
            format!("algorithm:{}", algorithm_type),
            if current_state == "Critical" {
                AuditResult::Failure
            } else {
                AuditResult::Partial
            },
        )
        .with_details(serde_json::json!({
            "previous_state": previous_state,
            "current_state": current_state,
            "algorithm_type": algorithm_type,
            "details": details
        }));

        self.log(event).await;
    }

    pub async fn log_rate_limit_exceeded(
        &self,
        workspace_id: Option<String>,
        client_ip: String,
        endpoint: String,
    ) {
        let event = AuditEvent::new(
            AuditEventType::RateLimitExceeded,
            workspace_id,
            "rate_limit_exceeded".to_string(),
            endpoint,
            AuditResult::Failure,
        )
        .with_client_ip(client_ip)
        .with_error("Rate limit exceeded".to_string());

        self.log(event).await;
    }

    pub async fn log_workspace_created(
        &self,
        workspace_id: String,
        workspace_name: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::WorkspaceCreated,
            Some(workspace_id),
            "create_workspace".to_string(),
            format!("workspace:{}", workspace_name),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_workspace_updated(
        &self,
        workspace_id: String,
        workspace_name: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::WorkspaceUpdated,
            Some(workspace_id),
            "update_workspace".to_string(),
            format!("workspace:{}", workspace_name),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_workspace_deleted(
        &self,
        workspace_id: String,
        workspace_name: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::WorkspaceDeleted,
            Some(workspace_id),
            "delete_workspace".to_string(),
            format!("workspace:{}", workspace_name),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_group_created(
        &self,
        workspace_id: String,
        group_id: String,
        group_name: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::GroupCreated,
            Some(workspace_id),
            "create_group".to_string(),
            format!("group:{}:{}", group_id, group_name),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_group_updated(
        &self,
        workspace_id: String,
        group_id: String,
        group_name: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::GroupUpdated,
            Some(workspace_id),
            "update_group".to_string(),
            format!("group:{}:{}", group_id, group_name),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_group_deleted(
        &self,
        workspace_id: String,
        group_id: String,
        group_name: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::GroupDeleted,
            Some(workspace_id),
            "delete_group".to_string(),
            format!("group:{}:{}", group_id, group_name),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_biz_tag_created(
        &self,
        workspace_id: String,
        biz_tag_id: String,
        biz_tag_name: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::BizTagCreated,
            Some(workspace_id),
            "create_biz_tag".to_string(),
            format!("biz_tag:{}:{}", biz_tag_id, biz_tag_name),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_biz_tag_updated(
        &self,
        workspace_id: String,
        biz_tag_id: String,
        biz_tag_name: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::BizTagUpdated,
            Some(workspace_id),
            "update_biz_tag".to_string(),
            format!("biz_tag:{}:{}", biz_tag_id, biz_tag_name),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_biz_tag_deleted(
        &self,
        workspace_id: String,
        biz_tag_id: String,
        biz_tag_name: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::BizTagDeleted,
            Some(workspace_id),
            "delete_biz_tag".to_string(),
            format!("biz_tag:{}:{}", biz_tag_id, biz_tag_name),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_api_key_created(
        &self,
        workspace_id: Option<String>,
        key_id: String,
        key_role: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ApiKeyCreated,
            workspace_id,
            "create_api_key".to_string(),
            format!("api_key:{}:{}", key_id, key_role),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_api_key_updated(
        &self,
        workspace_id: Option<String>,
        key_id: String,
        key_role: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ApiKeyUpdated,
            workspace_id,
            "update_api_key".to_string(),
            format!("api_key:{}:{}", key_id, key_role),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_api_key_deleted(
        &self,
        workspace_id: Option<String>,
        key_id: String,
        key_role: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ApiKeyDeleted,
            workspace_id,
            "delete_api_key".to_string(),
            format!("api_key:{}:{}", key_id, key_role),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn log_api_key_regenerated(
        &self,
        workspace_id: String,
        key_id: String,
        key_role: String,
        user_id: Option<String>,
        client_ip: Option<String>,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ApiKeyRegenerated,
            Some(workspace_id),
            "regenerate_api_key".to_string(),
            format!("api_key:{}:{}", key_id, key_role),
            AuditResult::Success,
        )
        .with_user_id(user_id.unwrap_or_default())
        .with_client_ip(client_ip.unwrap_or_default());

        self.log(event).await;
    }

    pub async fn get_recent_events(&self, limit: usize) -> Vec<AuditEvent> {
        let events = self.events.lock().await;
        events.iter().rev().take(limit).cloned().collect()
    }

    pub async fn get_events_by_workspace(&self, workspace_id: &str) -> Vec<AuditEvent> {
        let events = self.events.lock().await;
        events
            .iter()
            .filter(|e| e.workspace_id.as_deref() == Some(workspace_id))
            .cloned()
            .collect()
    }

    pub async fn get_events_by_type(&self, event_type: AuditEventType) -> Vec<AuditEvent> {
        let events = self.events.lock().await;
        events
            .iter()
            .filter(|e| e.event_type == event_type)
            .cloned()
            .collect()
    }

    pub fn total_logged(&self) -> u64 {
        self.total_logged.load(Ordering::SeqCst)
    }

    pub fn total_errors(&self) -> u64 {
        self.total_errors.load(Ordering::SeqCst)
    }

    pub fn get_total_logged(&self) -> u64 {
        self.total_logged.load(Ordering::SeqCst)
    }

    pub async fn clear(&self) {
        let mut events = self.events.lock().await;
        events.clear();
    }
}

#[async_trait]
impl CoreAuditLoggerTrait for AuditLogger {
    async fn log(&self, event: CoreAuditEvent) {
        // M4 修复：AuditEventType/AuditResult 已统一为 core 层定义，
        // 无需 match 转换。CoreAuditEvent.result 可能是 Unknown，
        // server 端保留该语义（不再强制映射为 Failure，避免信息丢失）。
        let server_event = AuditEvent {
            id: next_audit_event_id(),
            timestamp: event.timestamp,
            event_type: event.event_type,
            workspace_id: event.workspace_id,
            user_id: None,
            action: event.action,
            resource: event.resource,
            result: event.result,
            details: event.details,
            client_ip: None,
            user_agent: None,
            duration_ms: 0,
            error_message: None,
        };

        AuditLogger::log(self, server_event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_audit_logger_creation() {
        let logger = AuditLogger::new(100);
        assert_eq!(logger.total_logged(), 0);
    }

    #[tokio::test]
    async fn test_audit_event_logging() {
        let logger = AuditLogger::new(10);

        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            Some("test-workspace".to_string()),
            "generate_id".to_string(),
            "biz_tag:test".to_string(),
            AuditResult::Success,
        );

        logger.log(event.clone()).await;
        assert_eq!(logger.total_logged(), 1);

        let recent = logger.get_recent_events(5).await;
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].event_type, AuditEventType::IdGeneration);
    }

    #[tokio::test]
    async fn test_audit_event_details() {
        let logger = AuditLogger::new(10);

        let event = AuditEvent::new(
            AuditEventType::BatchGeneration,
            Some("test-workspace".to_string()),
            "batch_generate".to_string(),
            "biz_tag:test size:100".to_string(),
            AuditResult::Success,
        )
        .with_details(serde_json::json!({"batch_size": 100}))
        .with_client_ip("192.168.1.1".to_string())
        .with_duration(50);

        logger.log(event).await;

        let recent = logger.get_recent_events(1).await;
        assert!(recent[0].details.is_some());
        assert_eq!(recent[0].client_ip, Some("192.168.1.1".to_string()));
        assert_eq!(recent[0].duration_ms, 50);
    }

    #[tokio::test]
    async fn test_get_events_by_workspace() {
        let logger = AuditLogger::new(100);

        for i in 0..5 {
            let event = AuditEvent::new(
                AuditEventType::IdGeneration,
                Some("workspace-1".to_string()),
                format!("action-{}", i),
                "resource".to_string(),
                AuditResult::Success,
            );
            logger.log(event).await;
        }

        for i in 0..3 {
            let event = AuditEvent::new(
                AuditEventType::IdGeneration,
                Some("workspace-2".to_string()),
                format!("action-{}", i),
                "resource".to_string(),
                AuditResult::Success,
            );
            logger.log(event).await;
        }

        let workspace1_events = logger.get_events_by_workspace("workspace-1").await;
        assert_eq!(workspace1_events.len(), 5);

        let workspace2_events = logger.get_events_by_workspace("workspace-2").await;
        assert_eq!(workspace2_events.len(), 3);
    }

    #[tokio::test]
    async fn test_log_id_generation() {
        let logger = AuditLogger::new(10);

        logger
            .log_id_generation(
                "workspace-1".to_string(),
                "test-tag".to_string(),
                "123456789".to_string(),
                "snowflake".to_string(),
                Some("192.168.1.1".to_string()),
                5,
                true,
                None,
            )
            .await;

        assert_eq!(logger.total_logged(), 1);

        let events = logger
            .get_events_by_type(AuditEventType::IdGeneration)
            .await;
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_log_auth_event() {
        let logger = AuditLogger::new(10);

        logger
            .log_auth_event(
                Some("workspace-1".to_string()),
                "api_key_validate".to_string(),
                true,
                Some("192.168.1.1".to_string()),
                None,
            )
            .await;

        logger
            .log_auth_event(
                None,
                "api_key_validate".to_string(),
                false,
                Some("192.168.1.2".to_string()),
                Some("Invalid API key".to_string()),
            )
            .await;

        assert_eq!(logger.total_logged(), 2);

        let auth_events = logger
            .get_events_by_type(AuditEventType::Authentication)
            .await;
        assert_eq!(auth_events.len(), 2);
        assert_eq!(auth_events[0].result, AuditResult::Success);
        assert_eq!(auth_events[1].result, AuditResult::Failure);
        assert_eq!(
            auth_events[1].error_message,
            Some("Invalid API key".to_string())
        );
    }

    // ========== redact_ip branch coverage ==========

    #[test]
    fn test_redact_ip_ipv4_masks_last_octet() {
        let redacted = AuditEvent::redact_ip("192.168.1.100");
        assert_eq!(redacted, "192.168.1.x");
    }

    #[test]
    fn test_redact_ip_ipv4_loopback() {
        let redacted = AuditEvent::redact_ip("127.0.0.1");
        assert_eq!(redacted, "127.0.0.x");
    }

    #[test]
    fn test_redact_ip_ipv6_keeps_first_four_segments() {
        let redacted = AuditEvent::redact_ip("2001:db8:85a3:8d3:1319:8a2e:370a:7348");
        assert_eq!(redacted, "2001:db8:85a3:8d3:x:x:x:x");
    }

    #[test]
    fn test_redact_ip_ipv6_loopback() {
        let redacted = AuditEvent::redact_ip("::1");
        // ::1 parses as IPv6 with all-zero segments except last
        assert!(redacted.ends_with(":x:x:x:x"));
        assert!(redacted.starts_with("0:0:0:0"));
    }

    #[test]
    fn test_redact_ip_short_non_ip_kept_as_is() {
        // <= 8 chars non-IP: returned as-is (prefix)
        let redacted = AuditEvent::redact_ip("unknown");
        assert_eq!(redacted, "unknown");
    }

    #[test]
    fn test_redact_ip_long_non_ip_truncated_with_redacted_suffix() {
        // > 8 chars non-IP: first 8 chars + "...redacted"
        let redacted = AuditEvent::redact_ip("very-long-hostname.example.com");
        assert_eq!(redacted, "very-lon...redacted");
    }

    #[test]
    fn test_redact_ip_empty_string() {
        // Empty string: not a valid IP, prefix is empty, len <= 8 → empty string
        let redacted = AuditEvent::redact_ip("");
        assert_eq!(redacted, "");
    }

    // ========== Builder methods coverage ==========

    #[test]
    fn test_with_user_agent_sets_user_agent() {
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        )
        .with_user_agent("Mozilla/5.0".to_string());

        assert_eq!(event.user_agent.as_deref(), Some("Mozilla/5.0"));
    }

    #[test]
    fn test_with_user_id_sets_user_id() {
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        )
        .with_user_id("user-42".to_string());

        assert_eq!(event.user_id.as_deref(), Some("user-42"));
    }

    #[test]
    fn test_with_error_sets_error_message() {
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Failure,
        )
        .with_error("boom".to_string());

        assert_eq!(event.error_message.as_deref(), Some("boom"));
    }

    #[test]
    fn test_audit_event_full_builder_chain() {
        let event = AuditEvent::new(
            AuditEventType::ConfigChange,
            Some("ws".to_string()),
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        )
        .with_details(serde_json::json!({"k": "v"}))
        .with_client_ip("10.0.0.1".to_string())
        .with_user_agent("UA/1".to_string())
        .with_user_id("u1".to_string())
        .with_duration(42)
        .with_error("err".to_string());

        assert_eq!(event.event_type, AuditEventType::ConfigChange);
        assert_eq!(event.workspace_id.as_deref(), Some("ws"));
        assert_eq!(event.user_id.as_deref(), Some("u1"));
        assert_eq!(event.client_ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(event.user_agent.as_deref(), Some("UA/1"));
        assert_eq!(event.duration_ms, 42);
        assert_eq!(event.error_message.as_deref(), Some("err"));
        assert!(event.details.is_some());
    }

    // ========== validate_log_path branch coverage ==========

    #[test]
    fn test_validate_log_path_empty_rejected() {
        let result = AuditLogger::validate_log_path("");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_validate_log_path_whitespace_only_rejected() {
        let result = AuditLogger::validate_log_path("   ");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_validate_log_path_traversal_rejected() {
        let result = AuditLogger::validate_log_path("../../etc/passwd");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_validate_log_path_traversal_in_middle_rejected() {
        let result = AuditLogger::validate_log_path("logs/../audit.log");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_validate_log_path_relative_accepted() {
        let result = AuditLogger::validate_log_path("logs/audit.log");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_log_path_absolute_accepted() {
        // Use a platform-agnostic absolute path check
        let path = if cfg!(windows) {
            "C:\\logs\\audit.log"
        } else {
            "/var/log/nebulaid/audit.log"
        };
        let result = AuditLogger::validate_log_path(path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_log_path_filename_only_accepted() {
        let result = AuditLogger::validate_log_path("audit.log");
        assert!(result.is_ok());
    }

    // ========== Utility methods coverage ==========

    #[tokio::test]
    async fn test_total_errors_starts_at_zero() {
        let logger = AuditLogger::new(10);
        assert_eq!(logger.total_errors(), 0);
    }

    #[tokio::test]
    async fn test_clear_empties_events() {
        let logger = AuditLogger::new(10);
        for i in 0..5 {
            let event = AuditEvent::new(
                AuditEventType::IdGeneration,
                Some("ws".to_string()),
                format!("act-{i}"),
                "res".to_string(),
                AuditResult::Success,
            );
            logger.log(event).await;
        }
        assert_eq!(logger.total_logged(), 5);
        assert_eq!(logger.get_recent_events(100).await.len(), 5);

        logger.clear().await;

        // clear only empties the events buffer; total_logged counter unchanged
        assert_eq!(logger.get_recent_events(100).await.len(), 0);
        assert_eq!(logger.total_logged(), 5);
    }

    #[tokio::test]
    async fn test_get_recent_events_limit_zero_returns_empty() {
        let logger = AuditLogger::new(10);
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger.log(event).await;

        let recent = logger.get_recent_events(0).await;
        assert!(recent.is_empty());
    }

    #[tokio::test]
    async fn test_get_recent_events_returns_newest_first() {
        let logger = AuditLogger::new(10);
        for i in 0..3 {
            let event = AuditEvent::new(
                AuditEventType::IdGeneration,
                None,
                format!("act-{i}"),
                "res".to_string(),
                AuditResult::Success,
            );
            logger.log(event).await;
        }

        let recent = logger.get_recent_events(2).await;
        assert_eq!(recent.len(), 2);
        // Newest first: act-2 was logged last, so it's first in the result
        assert_eq!(recent[0].action, "act-2");
        assert_eq!(recent[1].action, "act-1");
    }

    #[tokio::test]
    async fn test_get_events_by_workspace_no_match_returns_empty() {
        let logger = AuditLogger::new(10);
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            Some("ws-1".to_string()),
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger.log(event).await;

        let events = logger.get_events_by_workspace("nonexistent").await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_get_events_by_workspace_filters_none_workspace() {
        let logger = AuditLogger::new(10);
        // Event with no workspace_id
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger.log(event).await;

        let events = logger.get_events_by_workspace("any").await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_get_events_by_type_no_match_returns_empty() {
        let logger = AuditLogger::new(10);
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger.log(event).await;

        let events = logger
            .get_events_by_type(AuditEventType::Authentication)
            .await;
        assert!(events.is_empty());
    }

    #[test]
    fn test_next_audit_event_id_generates_unique_ids() {
        let id1 = next_audit_event_id();
        let id2 = next_audit_event_id();
        let id3 = next_audit_event_id();
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_next_audit_event_id_uses_high_44_bits_for_timestamp() {
        // Low 20 bits are counter; high 44 bits should be >= some recent unix_ms
        let id = next_audit_event_id();
        let high_44 = id >> 20;
        // Unix millis for 2024-01-01 is around 1704067200000
        assert!(high_44 >= 1704067200000);
    }

    #[test]
    fn test_audit_event_new_defaults() {
        let event = AuditEvent::new(
            AuditEventType::HealthCheck,
            Some("ws".to_string()),
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        assert_eq!(event.event_type, AuditEventType::HealthCheck);
        assert_eq!(event.workspace_id.as_deref(), Some("ws"));
        assert!(event.user_id.is_none());
        assert!(event.details.is_none());
        assert!(event.client_ip.is_none());
        assert!(event.user_agent.is_none());
        assert_eq!(event.duration_ms, 0);
        assert!(event.error_message.is_none());
    }

    // ========== log_batch_generation coverage ==========

    #[tokio::test]
    async fn test_log_batch_generation_success_with_client_ip() {
        let logger = AuditLogger::new(10);
        logger
            .log_batch_generation(
                "ws-1".to_string(),
                "tag-1".to_string(),
                100,
                Some("10.0.0.1".to_string()),
                25,
                true,
                None,
            )
            .await;

        assert_eq!(logger.total_logged(), 1);
        let events = logger
            .get_events_by_type(AuditEventType::BatchGeneration)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].result, AuditResult::Success);
        assert_eq!(events[0].action, "batch_generate_ids");
        assert!(events[0].resource.contains("tag-1"));
        assert!(events[0].resource.contains("100"));
        assert_eq!(events[0].client_ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(events[0].duration_ms, 25);
        assert!(events[0].error_message.is_none());
        // Verify details payload
        let details = events[0].details.as_ref().expect("details should be set");
        assert_eq!(details["batch_size"], 100);
        assert_eq!(details["biz_tag"], "tag-1");
    }

    #[tokio::test]
    async fn test_log_batch_generation_failure_with_error_message() {
        let logger = AuditLogger::new(10);
        logger
            .log_batch_generation(
                "ws-1".to_string(),
                "tag-1".to_string(),
                50,
                None,
                10,
                false,
                Some("database unavailable".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::BatchGeneration)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].result, AuditResult::Failure);
        assert_eq!(events[0].client_ip.as_deref(), Some(""));
        assert_eq!(
            events[0].error_message.as_deref(),
            Some("database unavailable")
        );
    }

    #[tokio::test]
    async fn test_log_batch_generation_success_no_error_no_client_ip() {
        // Covers: success=true branch, error_message=None branch, client_ip=None → unwrap_or_default
        let logger = AuditLogger::new(10);
        logger
            .log_batch_generation(
                "ws-2".to_string(),
                "tag-2".to_string(),
                1,
                None,
                0,
                true,
                None,
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::BatchGeneration)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].result, AuditResult::Success);
        assert!(events[0].error_message.is_none());
    }

    // ========== log_config_change coverage ==========

    #[tokio::test]
    async fn test_log_config_change_with_workspace() {
        let logger = AuditLogger::new(10);
        logger
            .log_config_change(
                Some("ws-1".to_string()),
                "update_rate_limit".to_string(),
                "biz_tag".to_string(),
                serde_json::json!({"tag": "t1", "old": 100, "new": 200}),
            )
            .await;

        assert_eq!(logger.total_logged(), 1);
        let events = logger
            .get_events_by_type(AuditEventType::ConfigChange)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "update_rate_limit");
        assert_eq!(events[0].resource, "config:biz_tag");
        assert_eq!(events[0].result, AuditResult::Success);
        assert_eq!(events[0].workspace_id.as_deref(), Some("ws-1"));
        assert!(events[0].details.is_some());
        assert_eq!(events[0].details.as_ref().unwrap()["new"], 200);
    }

    #[tokio::test]
    async fn test_log_config_change_without_workspace() {
        let logger = AuditLogger::new(10);
        logger
            .log_config_change(
                None,
                "global_update".to_string(),
                "system".to_string(),
                serde_json::json!({"key": "value"}),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::ConfigChange)
            .await;
        assert_eq!(events.len(), 1);
        assert!(events[0].workspace_id.is_none());
        assert_eq!(events[0].resource, "config:system");
    }

    // ========== log_degradation_event coverage ==========

    #[tokio::test]
    async fn test_log_degradation_event_critical_state_is_failure() {
        let logger = AuditLogger::new(10);
        logger
            .log_degradation_event(
                Some("ws-1".to_string()),
                "circuit_breaker_open".to_string(),
                "snowflake".to_string(),
                "Normal".to_string(),
                "Critical".to_string(),
                serde_json::json!({"consecutive_failures": 5}),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::DegradationEvent)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].result, AuditResult::Failure);
        assert_eq!(events[0].resource, "algorithm:snowflake");
        let details = events[0].details.as_ref().unwrap();
        assert_eq!(details["previous_state"], "Normal");
        assert_eq!(details["current_state"], "Critical");
        assert_eq!(details["algorithm_type"], "snowflake");
    }

    #[tokio::test]
    async fn test_log_degradation_event_non_critical_state_is_partial() {
        let logger = AuditLogger::new(10);
        logger
            .log_degradation_event(
                None,
                "degraded_mode".to_string(),
                "segment".to_string(),
                "Normal".to_string(),
                "Degraded".to_string(),
                serde_json::json!({}),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::DegradationEvent)
            .await;
        assert_eq!(events.len(), 1);
        // Non-Critical → Partial (per server-side rule; core layer maps Normal→Success,
        // but server logger only special-cases "Critical")
        assert_eq!(events[0].result, AuditResult::Partial);
    }

    #[tokio::test]
    async fn test_log_degradation_event_normal_state_is_partial() {
        // Server-side log_degradation_event only checks == "Critical"; everything else → Partial.
        let logger = AuditLogger::new(10);
        logger
            .log_degradation_event(
                Some("ws-1".to_string()),
                "recovered".to_string(),
                "uuid_v7".to_string(),
                "Critical".to_string(),
                "Normal".to_string(),
                serde_json::json!({"recovered_at": "2026-07-20"}),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::DegradationEvent)
            .await;
        assert_eq!(events[0].result, AuditResult::Partial);
    }

    // ========== log_rate_limit_exceeded coverage ==========

    #[tokio::test]
    async fn test_log_rate_limit_exceeded_with_workspace() {
        let logger = AuditLogger::new(10);
        logger
            .log_rate_limit_exceeded(
                Some("ws-1".to_string()),
                "203.0.113.5".to_string(),
                "/api/v1/ids/generate".to_string(),
            )
            .await;

        assert_eq!(logger.total_logged(), 1);
        let events = logger
            .get_events_by_type(AuditEventType::RateLimitExceeded)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].result, AuditResult::Failure);
        assert_eq!(events[0].action, "rate_limit_exceeded");
        assert_eq!(events[0].resource, "/api/v1/ids/generate");
        assert_eq!(events[0].workspace_id.as_deref(), Some("ws-1"));
        assert_eq!(events[0].client_ip.as_deref(), Some("203.0.113.5"));
        assert_eq!(
            events[0].error_message.as_deref(),
            Some("Rate limit exceeded")
        );
    }

    #[tokio::test]
    async fn test_log_rate_limit_exceeded_without_workspace() {
        let logger = AuditLogger::new(10);
        logger
            .log_rate_limit_exceeded(
                None,
                "198.51.100.7".to_string(),
                "/api/v1/batch".to_string(),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::RateLimitExceeded)
            .await;
        assert_eq!(events.len(), 1);
        assert!(events[0].workspace_id.is_none());
    }

    // ========== log_id_generation additional branches ==========

    #[tokio::test]
    async fn test_log_id_generation_failure_with_error() {
        // Covers: success=false branch, error_message=Some branch, client_ip=None branch
        let logger = AuditLogger::new(10);
        logger
            .log_id_generation(
                "ws-fail".to_string(),
                "tag-fail".to_string(),
                "".to_string(),
                "segment".to_string(),
                None,
                100,
                false,
                Some("segment exhausted".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::IdGeneration)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].result, AuditResult::Failure);
        assert_eq!(events[0].client_ip.as_deref(), Some(""));
        assert_eq!(
            events[0].error_message.as_deref(),
            Some("segment exhausted")
        );
        assert_eq!(events[0].duration_ms, 100);
        let details = events[0].details.as_ref().unwrap();
        assert_eq!(details["algorithm"], "segment");
    }

    // ========== log_workspace_* coverage ==========

    #[tokio::test]
    async fn test_log_workspace_created_with_user_and_ip() {
        let logger = AuditLogger::new(10);
        logger
            .log_workspace_created(
                "ws-1".to_string(),
                "My Workspace".to_string(),
                Some("user-1".to_string()),
                Some("10.0.0.1".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::WorkspaceCreated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "create_workspace");
        assert_eq!(events[0].resource, "workspace:My Workspace");
        assert_eq!(events[0].result, AuditResult::Success);
        assert_eq!(events[0].workspace_id.as_deref(), Some("ws-1"));
        assert_eq!(events[0].user_id.as_deref(), Some("user-1"));
        assert_eq!(events[0].client_ip.as_deref(), Some("10.0.0.1"));
    }

    #[tokio::test]
    async fn test_log_workspace_created_without_user_and_ip() {
        // Covers: user_id=None → unwrap_or_default, client_ip=None → unwrap_or_default
        let logger = AuditLogger::new(10);
        logger
            .log_workspace_created("ws-2".to_string(), "Empty".to_string(), None, None)
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::WorkspaceCreated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].user_id.as_deref(), Some(""));
        assert_eq!(events[0].client_ip.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn test_log_workspace_updated() {
        let logger = AuditLogger::new(10);
        logger
            .log_workspace_updated(
                "ws-1".to_string(),
                "Updated Name".to_string(),
                Some("user-2".to_string()),
                Some("10.0.0.2".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::WorkspaceUpdated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "update_workspace");
        assert_eq!(events[0].resource, "workspace:Updated Name");
        assert_eq!(events[0].result, AuditResult::Success);
    }

    #[tokio::test]
    async fn test_log_workspace_deleted() {
        let logger = AuditLogger::new(10);
        logger
            .log_workspace_deleted(
                "ws-del".to_string(),
                "Deleted".to_string(),
                Some("admin".to_string()),
                Some("10.0.0.3".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::WorkspaceDeleted)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "delete_workspace");
        assert_eq!(events[0].resource, "workspace:Deleted");
        assert_eq!(events[0].result, AuditResult::Success);
    }

    // ========== log_group_* coverage ==========

    #[tokio::test]
    async fn test_log_group_created() {
        let logger = AuditLogger::new(10);
        logger
            .log_group_created(
                "ws-1".to_string(),
                "grp-1".to_string(),
                "Engineering".to_string(),
                Some("user-1".to_string()),
                Some("10.0.0.1".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::GroupCreated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "create_group");
        assert_eq!(events[0].resource, "group:grp-1:Engineering");
        assert_eq!(events[0].result, AuditResult::Success);
        assert_eq!(events[0].workspace_id.as_deref(), Some("ws-1"));
        assert_eq!(events[0].user_id.as_deref(), Some("user-1"));
    }

    #[tokio::test]
    async fn test_log_group_updated() {
        let logger = AuditLogger::new(10);
        logger
            .log_group_updated(
                "ws-1".to_string(),
                "grp-1".to_string(),
                "Engineering Renamed".to_string(),
                None,
                None,
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::GroupUpdated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "update_group");
        assert_eq!(events[0].resource, "group:grp-1:Engineering Renamed");
        // None → unwrap_or_default → empty string
        assert_eq!(events[0].user_id.as_deref(), Some(""));
        assert_eq!(events[0].client_ip.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn test_log_group_deleted() {
        let logger = AuditLogger::new(10);
        logger
            .log_group_deleted(
                "ws-1".to_string(),
                "grp-del".to_string(),
                "Removed".to_string(),
                Some("admin".to_string()),
                Some("10.0.0.9".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::GroupDeleted)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "delete_group");
        assert_eq!(events[0].resource, "group:grp-del:Removed");
    }

    // ========== log_biz_tag_* coverage ==========

    #[tokio::test]
    async fn test_log_biz_tag_created() {
        let logger = AuditLogger::new(10);
        logger
            .log_biz_tag_created(
                "ws-1".to_string(),
                "tag-1".to_string(),
                "Order IDs".to_string(),
                Some("user-1".to_string()),
                Some("10.0.0.1".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::BizTagCreated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "create_biz_tag");
        assert_eq!(events[0].resource, "biz_tag:tag-1:Order IDs");
        assert_eq!(events[0].result, AuditResult::Success);
    }

    #[tokio::test]
    async fn test_log_biz_tag_updated() {
        let logger = AuditLogger::new(10);
        logger
            .log_biz_tag_updated(
                "ws-1".to_string(),
                "tag-1".to_string(),
                "Order IDs v2".to_string(),
                None,
                None,
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::BizTagUpdated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "update_biz_tag");
        assert_eq!(events[0].resource, "biz_tag:tag-1:Order IDs v2");
    }

    #[tokio::test]
    async fn test_log_biz_tag_deleted() {
        let logger = AuditLogger::new(10);
        logger
            .log_biz_tag_deleted(
                "ws-1".to_string(),
                "tag-del".to_string(),
                "Removed".to_string(),
                Some("admin".to_string()),
                Some("10.0.0.9".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::BizTagDeleted)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "delete_biz_tag");
        assert_eq!(events[0].resource, "biz_tag:tag-del:Removed");
    }

    // ========== log_api_key_* coverage ==========

    #[tokio::test]
    async fn test_log_api_key_created_with_workspace() {
        let logger = AuditLogger::new(10);
        logger
            .log_api_key_created(
                Some("ws-1".to_string()),
                "key-1".to_string(),
                "admin".to_string(),
                Some("user-1".to_string()),
                Some("10.0.0.1".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::ApiKeyCreated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "create_api_key");
        assert_eq!(events[0].resource, "api_key:key-1:admin");
        assert_eq!(events[0].result, AuditResult::Success);
        assert_eq!(events[0].workspace_id.as_deref(), Some("ws-1"));
    }

    #[tokio::test]
    async fn test_log_api_key_created_without_workspace() {
        // Admin keys can have workspace_id = None
        let logger = AuditLogger::new(10);
        logger
            .log_api_key_created(
                None,
                "admin-key".to_string(),
                "admin".to_string(),
                None,
                None,
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::ApiKeyCreated)
            .await;
        assert_eq!(events.len(), 1);
        assert!(events[0].workspace_id.is_none());
        assert_eq!(events[0].user_id.as_deref(), Some(""));
        assert_eq!(events[0].client_ip.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn test_log_api_key_updated() {
        let logger = AuditLogger::new(10);
        logger
            .log_api_key_updated(
                Some("ws-1".to_string()),
                "key-1".to_string(),
                "user".to_string(),
                Some("user-1".to_string()),
                Some("10.0.0.2".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::ApiKeyUpdated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "update_api_key");
        assert_eq!(events[0].resource, "api_key:key-1:user");
    }

    #[tokio::test]
    async fn test_log_api_key_deleted() {
        let logger = AuditLogger::new(10);
        logger
            .log_api_key_deleted(
                None,
                "key-del".to_string(),
                "admin".to_string(),
                Some("admin".to_string()),
                Some("10.0.0.9".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::ApiKeyDeleted)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "delete_api_key");
        assert_eq!(events[0].resource, "api_key:key-del:admin");
        assert!(events[0].workspace_id.is_none());
    }

    #[tokio::test]
    async fn test_log_api_key_regenerated() {
        let logger = AuditLogger::new(10);
        logger
            .log_api_key_regenerated(
                "ws-1".to_string(),
                "key-1".to_string(),
                "user".to_string(),
                Some("user-1".to_string()),
                Some("10.0.0.3".to_string()),
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::ApiKeyRegenerated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "regenerate_api_key");
        assert_eq!(events[0].resource, "api_key:key-1:user");
        assert_eq!(events[0].result, AuditResult::Success);
        assert_eq!(events[0].workspace_id.as_deref(), Some("ws-1"));
    }

    #[tokio::test]
    async fn test_log_api_key_regenerated_without_user_and_ip() {
        // Covers: user_id=None, client_ip=None branches
        let logger = AuditLogger::new(10);
        logger
            .log_api_key_regenerated(
                "ws-2".to_string(),
                "key-2".to_string(),
                "admin".to_string(),
                None,
                None,
            )
            .await;

        let events = logger
            .get_events_by_type(AuditEventType::ApiKeyRegenerated)
            .await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].user_id.as_deref(), Some(""));
        assert_eq!(events[0].client_ip.as_deref(), Some(""));
    }

    // ========== with_file_logging coverage ==========

    #[tokio::test]
    async fn test_with_file_logging_empty_path_falls_back_to_memory_logger() {
        // Invalid path → validate_log_path fails → fall back to new(max_events)
        let logger = AuditLogger::with_file_logging(100, "".to_string()).await;
        // Fall-back logger has no file_tx
        assert_eq!(logger.total_logged(), 0);
        assert_eq!(logger.total_errors(), 0);
        // Verify it still works as a memory logger
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger.log(event).await;
        assert_eq!(logger.total_logged(), 1);
    }

    #[tokio::test]
    async fn test_with_file_logging_traversal_path_falls_back_to_memory_logger() {
        let logger = AuditLogger::with_file_logging(100, "../../etc/passwd".to_string()).await;
        // Should fall back — no file persistence, but logger still functional
        assert_eq!(logger.total_errors(), 0);
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger.log(event).await;
        assert_eq!(logger.total_logged(), 1);
    }

    #[tokio::test]
    async fn test_with_file_logging_valid_path_persists_events() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let log_path = tmp.path().join("audit.log");
        let path_str = log_path.to_str().expect("path is utf-8").to_string();

        let logger = AuditLogger::with_file_logging(100, path_str.clone()).await;
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            Some("ws-1".to_string()),
            "generate_id".to_string(),
            "biz_tag:test".to_string(),
            AuditResult::Success,
        )
        .with_client_ip("192.168.1.100".to_string())
        .with_user_agent("Mozilla/5.0".to_string())
        .with_duration(15);

        logger.log(event).await;

        // Give the writer task time to flush
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Read file and verify content
        let content = tokio::fs::read_to_string(&log_path)
            .await
            .expect("file should exist");
        assert!(!content.is_empty());
        // Each event is one JSON line
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 1);

        let parsed: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON");
        assert_eq!(parsed["action"], "generate_id");
        assert_eq!(parsed["workspace_id"], "ws-1");
        assert_eq!(parsed["duration_ms"], 15);
        // PII redaction: client_ip last octet masked
        assert_eq!(parsed["client_ip"], "192.168.1.x");
        // user_agent replaced with fixed string
        assert_eq!(parsed["user_agent"], "UA(redacted)");
    }

    #[tokio::test]
    async fn test_with_file_logging_persists_multiple_events() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let log_path = tmp.path().join("multi.log");
        let path_str = log_path.to_str().expect("path is utf-8").to_string();

        let logger = AuditLogger::with_file_logging(100, path_str.clone()).await;
        for i in 0..5 {
            let event = AuditEvent::new(
                AuditEventType::IdGeneration,
                Some("ws-batch".to_string()),
                format!("act-{i}"),
                "res".to_string(),
                AuditResult::Success,
            );
            logger.log(event).await;
        }

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let content = tokio::fs::read_to_string(&log_path)
            .await
            .expect("file should exist");
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 5);
        // Verify each line is valid JSON
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
            assert_eq!(parsed["workspace_id"], "ws-batch");
        }
        assert_eq!(logger.total_logged(), 5);
        assert_eq!(logger.total_errors(), 0);
    }

    #[tokio::test]
    async fn test_with_file_logging_write_failure_increments_errors() {
        // Use a path where the parent directory doesn't exist — write_event_to_file
        // will fail with "No such file or directory", and the writer task increments
        // total_errors.
        let tmp = tempfile::tempdir().expect("create temp dir");
        let nonexistent = tmp.path().join("nonexistent_subdir").join("audit.log");
        let path_str = nonexistent.to_str().expect("path is utf-8").to_string();

        let logger = AuditLogger::with_file_logging(100, path_str).await;
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger.log(event).await;

        // Wait for writer task to attempt and fail
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(logger.total_errors(), 1);
    }

    // ========== write_event_to_file direct tests ==========

    #[tokio::test]
    async fn test_write_event_to_file_invalid_path_returns_err() {
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        // Path to a directory that doesn't exist
        let result = AuditLogger::write_event_to_file(&event, "/nonexistent_dir/audit.log").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_event_to_file_success_writes_json_line() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let log_path = tmp.path().join("direct.log");
        let path_str = log_path.to_str().expect("path is utf-8").to_string();

        let event = AuditEvent::new(
            AuditEventType::Authentication,
            Some("ws-1".to_string()),
            "login".to_string(),
            "auth".to_string(),
            AuditResult::Success,
        )
        .with_client_ip("10.20.30.40".to_string())
        .with_user_agent("curl/8.0".to_string())
        .with_user_id("user-99".to_string())
        .with_duration(5)
        .with_error("no error".to_string());

        AuditLogger::write_event_to_file(&event, &path_str)
            .await
            .expect("write should succeed");

        let content = std::fs::read_to_string(&log_path).expect("file should exist");
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).expect("valid JSON");
        assert_eq!(parsed["action"], "login");
        assert_eq!(parsed["event_type"], "Authentication");
        assert_eq!(parsed["user_id"], "user-99");
        assert_eq!(parsed["error_message"], "no error");
        assert_eq!(parsed["duration_ms"], 5);
        // PII redaction
        assert_eq!(parsed["client_ip"], "10.20.30.x");
        assert_eq!(parsed["user_agent"], "UA(redacted)");
    }

    #[tokio::test]
    async fn test_write_event_to_file_appends_to_existing() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let log_path = tmp.path().join("append.log");
        let path_str = log_path.to_str().expect("path is utf-8").to_string();

        for i in 0..3 {
            let event = AuditEvent::new(
                AuditEventType::IdGeneration,
                None,
                format!("act-{i}"),
                "res".to_string(),
                AuditResult::Success,
            );
            AuditLogger::write_event_to_file(&event, &path_str)
                .await
                .expect("write should succeed");
        }

        let content = std::fs::read_to_string(&log_path).expect("file should exist");
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 3);
    }

    #[tokio::test]
    async fn test_write_event_to_file_redacts_ipv6() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let log_path = tmp.path().join("ipv6.log");
        let path_str = log_path.to_str().expect("path is utf-8").to_string();

        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        )
        .with_client_ip("2001:db8::1".to_string())
        .with_user_agent("UA".to_string());

        AuditLogger::write_event_to_file(&event, &path_str)
            .await
            .expect("write should succeed");

        let content = std::fs::read_to_string(&log_path).expect("file should exist");
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).expect("valid JSON");
        // IPv6: first 4 segments kept, rest masked
        let ip = parsed["client_ip"].as_str().expect("client_ip is string");
        assert!(ip.ends_with(":x:x:x:x"));
    }

    // ========== VecDeque overflow coverage ==========

    #[tokio::test]
    async fn test_log_drops_oldest_without_persistence_increments_errors() {
        // max_events = 1, no file_tx → second log drops first event, increments total_errors
        let logger = AuditLogger::new(1);
        let e1 = AuditEvent::new(
            AuditEventType::IdGeneration,
            Some("ws".to_string()),
            "first".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        let e2 = AuditEvent::new(
            AuditEventType::IdGeneration,
            Some("ws".to_string()),
            "second".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger.log(e1).await;
        logger.log(e2).await;

        // Both events were "logged" (total_logged incremented)
        assert_eq!(logger.total_logged(), 2);
        // First event was dropped without persistence → error
        assert_eq!(logger.total_errors(), 1);
        // Only the second event remains in memory
        let recent = logger.get_recent_events(10).await;
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].action, "second");
    }

    #[tokio::test]
    async fn test_log_drops_oldest_with_persistence_no_error() {
        // max_events = 1, with file_tx → second log drops first event, but it was
        // already written to file, so no error.
        let tmp = tempfile::tempdir().expect("create temp dir");
        let log_path = tmp.path().join("overflow.log");
        let path_str = log_path.to_str().expect("path is utf-8").to_string();

        let logger = AuditLogger::with_file_logging(1, path_str.clone()).await;
        let e1 = AuditEvent::new(
            AuditEventType::IdGeneration,
            Some("ws".to_string()),
            "first".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        let e2 = AuditEvent::new(
            AuditEventType::IdGeneration,
            Some("ws".to_string()),
            "second".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger.log(e1).await;
        logger.log(e2).await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Both events logged
        assert_eq!(logger.total_logged(), 2);
        // No errors because file_tx is Some (dropped events were persisted)
        assert_eq!(logger.total_errors(), 0);
        // Only second event in memory
        let recent = logger.get_recent_events(10).await;
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].action, "second");
        // Both events in file
        let content = tokio::fs::read_to_string(&log_path)
            .await
            .expect("file should exist");
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 2);
    }

    #[tokio::test]
    async fn test_log_drops_multiple_when_far_over_capacity() {
        // max_events = 2, log 5 events → 3 dropped
        let logger = AuditLogger::new(2);
        for i in 0..5 {
            let event = AuditEvent::new(
                AuditEventType::IdGeneration,
                Some("ws".to_string()),
                format!("act-{i}"),
                "res".to_string(),
                AuditResult::Success,
            );
            logger.log(event).await;
        }

        assert_eq!(logger.total_logged(), 5);
        // 3 events dropped without persistence → 3 errors
        assert_eq!(logger.total_errors(), 3);
        // Only last 2 remain in memory
        let recent = logger.get_recent_events(10).await;
        assert_eq!(recent.len(), 2);
        // Newest first
        assert_eq!(recent[0].action, "act-4");
        assert_eq!(recent[1].action, "act-3");
    }

    // ========== CoreAuditLoggerTrait impl coverage ==========

    #[tokio::test]
    async fn test_core_audit_logger_trait_log_converts_and_logs() {
        use crate::core::algorithm::AuditEvent as CoreAuditEvent;

        let logger = AuditLogger::new(10);
        let core_event = CoreAuditEvent::new(
            AuditEventType::DegradationEvent,
            Some("ws-core".to_string()),
            "degrade".to_string(),
            "algorithm:segment".to_string(),
            AuditResult::Partial,
        )
        .with_details(serde_json::json!({"reason": "circuit_open"}));

        // Use the trait method (not the inherent method)
        use crate::core::algorithm::AuditLogger as CoreAuditLoggerTrait;
        CoreAuditLoggerTrait::log(&logger, core_event).await;

        assert_eq!(logger.total_logged(), 1);
        let events = logger.get_recent_events(1).await;
        assert_eq!(events.len(), 1);
        // Server-side AuditEvent has id (from next_audit_event_id), client_ip=None,
        // user_agent=None, duration_ms=0, error_message=None
        assert!(events[0].id > 0);
        assert!(events[0].client_ip.is_none());
        assert!(events[0].user_agent.is_none());
        assert_eq!(events[0].duration_ms, 0);
        assert!(events[0].error_message.is_none());
        assert_eq!(events[0].workspace_id.as_deref(), Some("ws-core"));
        assert_eq!(events[0].result, AuditResult::Partial);
        assert_eq!(events[0].event_type, AuditEventType::DegradationEvent);
        // details carried over from core event
        assert!(events[0].details.is_some());
    }

    #[tokio::test]
    async fn test_core_audit_logger_trait_log_unknown_result_preserved() {
        // CoreAuditEvent can have result=Unknown; server-side preserves it
        // (M4 fix: no longer forced to Failure).
        use crate::core::algorithm::{
            AuditEvent as CoreAuditEvent, AuditLogger as CoreAuditLoggerTrait,
        };

        let logger = AuditLogger::new(10);
        let core_event = CoreAuditEvent {
            event_type: AuditEventType::HealthCheck,
            workspace_id: None,
            action: "check".to_string(),
            resource: "health".to_string(),
            result: AuditResult::Unknown,
            details: None,
            timestamp: chrono::Utc::now(),
        };

        CoreAuditLoggerTrait::log(&logger, core_event).await;

        let events = logger.get_recent_events(1).await;
        assert_eq!(events[0].result, AuditResult::Unknown);
    }

    // ========== log with closed file channel ==========

    #[tokio::test]
    async fn test_log_with_closed_file_channel_increments_errors() {
        // Construct a logger manually with a dead sender (receiver dropped).
        // Tests the `tx.send().is_err()` branch in log().
        let (tx, rx) = mpsc::unbounded_channel::<AuditEvent>();
        drop(rx); // Close the channel

        let logger = AuditLogger {
            events: Arc::new(Mutex::new(VecDeque::with_capacity(10))),
            max_events: 10,
            total_logged: Arc::new(AtomicU64::new(0)),
            total_errors: Arc::new(AtomicU64::new(0)),
            file_tx: Some(tx),
            _writer_task: None,
        };

        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            None,
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger.log(event).await;

        // Event was still logged to memory
        assert_eq!(logger.total_logged(), 1);
        // Channel closed → error incremented
        assert_eq!(logger.total_errors(), 1);
    }

    // ========== Clone semantics ==========

    #[tokio::test]
    async fn test_logger_clone_shares_state() {
        // AuditLogger derives Clone; cloned loggers should share the same
        // events buffer and counters (Arc-backed).
        let logger = AuditLogger::new(10);
        let logger_clone = logger.clone();

        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            Some("ws".to_string()),
            "act".to_string(),
            "res".to_string(),
            AuditResult::Success,
        );
        logger_clone.log(event).await;

        // Original logger sees the event (shared Arc<Mutex>)
        assert_eq!(logger.total_logged(), 1);
        let recent = logger.get_recent_events(10).await;
        assert_eq!(recent.len(), 1);
    }
}
