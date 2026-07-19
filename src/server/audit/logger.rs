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
}
