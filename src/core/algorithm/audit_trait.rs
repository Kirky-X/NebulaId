// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventType {
    // 核心算法事件
    IdGeneration,
    BatchGeneration,
    Authentication,
    ConfigChange,
    DegradationEvent,
    RateLimitExceeded,
    HealthCheck,
    MetricsAccess,
    // 业务管理事件
    WorkspaceCreated,
    WorkspaceUpdated,
    WorkspaceDeleted,
    GroupCreated,
    GroupUpdated,
    GroupDeleted,
    BizTagCreated,
    BizTagUpdated,
    BizTagDeleted,
    ApiKeyCreated,
    ApiKeyUpdated,
    ApiKeyDeleted,
    ApiKeyRegenerated,
    /// 请求级降级命中：主算法不可用，本次生成由备用算法完成。
    FallbackUsed,
    /// 时钟回拨安全事件：Snowflake 生成路径检测到时钟倒退。
    ClockBackwards,
}

/// 时钟回拨的处置结果：阈值内等待后继续，或超阈值报错。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClockResolution {
    /// 阈值内等待后恢复，生成成功。
    Waited,
    /// 超阈值，生成失败并报 `CoreError::ClockMovedBackward`。
    Errored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditResult {
    Success,
    Failure,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_type: AuditEventType,
    pub workspace_id: Option<String>,
    pub action: String,
    pub resource: String,
    pub result: AuditResult,
    pub details: Option<serde_json::Value>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
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
            event_type,
            workspace_id,
            action,
            resource,
            result,
            details: None,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

#[async_trait]
pub trait AuditLogger: Send + Sync {
    async fn log(&self, event: AuditEvent);

    async fn log_id_generation(
        &self,
        workspace_id: Option<String>,
        action: String,
        algorithm_type: String,
        id: String,
        success: bool,
    ) {
        let event = AuditEvent::new(
            AuditEventType::IdGeneration,
            workspace_id,
            action,
            format!("id:{}", id),
            if success {
                AuditResult::Success
            } else {
                AuditResult::Failure
            },
        )
        .with_details(serde_json::json!({
            "algorithm_type": algorithm_type
        }));
        self.log(event).await;
    }

    /// 时钟回拨事件：由路由层 observe 单点发射，记录幅度与处置结果。
    async fn log_clock_backwards(
        &self,
        workspace_id: Option<String>,
        algorithm: String,
        details: serde_json::Value,
        resolution: ClockResolution,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ClockBackwards,
            workspace_id,
            "clock_backwards".to_string(),
            format!("algorithm:{algorithm}"),
            if resolution == ClockResolution::Errored {
                AuditResult::Failure
            } else {
                AuditResult::Success
            },
        )
        .with_details(serde_json::json!({
            "resolution": resolution,
            "details": details,
        }));
        self.log(event).await;
    }

    async fn log_config_change(
        &self,
        workspace_id: Option<String>,
        action: String,
        resource: String,
        details: serde_json::Value,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ConfigChange,
            workspace_id,
            action,
            resource,
            AuditResult::Success,
        )
        .with_details(details);
        self.log(event).await;
    }

    async fn log_degradation_event(
        &self,
        workspace_id: Option<String>,
        action: String,
        algorithm_type: String,
        previous_state: String,
        current_state: String,
        details: serde_json::Value,
    ) {
        let result = match current_state.as_str() {
            "Critical" => AuditResult::Failure,
            "Normal" => AuditResult::Success,
            _ => AuditResult::Partial,
        };
        let event = AuditEvent::new(
            AuditEventType::DegradationEvent,
            workspace_id,
            action,
            format!("algorithm:{}", algorithm_type),
            result,
        )
        .with_details(serde_json::json!({
            "previous_state": previous_state,
            "current_state": current_state,
            "algorithm_type": algorithm_type,
            "details": details
        }));
        self.log(event).await;
    }
}

pub type DynAuditLogger = Arc<dyn AuditLogger>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingLogger {
        events: Mutex<Vec<AuditEvent>>,
    }

    #[async_trait]
    impl AuditLogger for RecordingLogger {
        async fn log(&self, event: AuditEvent) {
            self.events.lock().expect("lock").push(event);
        }
    }

    fn logger() -> RecordingLogger {
        RecordingLogger {
            events: Mutex::new(Vec::new()),
        }
    }

    #[tokio::test]
    async fn test_default_log_id_generation_records_success_and_failure() {
        let ok_logger = logger();
        ok_logger
            .log_id_generation(
                Some("ws".to_string()),
                "generate".to_string(),
                "snowflake".to_string(),
                "42".to_string(),
                true,
            )
            .await;
        {
            let events = ok_logger.events.lock().expect("lock");
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].result, AuditResult::Success);
            assert!(events[0].resource.contains("42"));
        }

        let fail_logger = logger();
        fail_logger
            .log_id_generation(
                None,
                "generate".to_string(),
                "segment".to_string(),
                "".to_string(),
                false,
            )
            .await;
        let events = fail_logger.events.lock().expect("lock");
        assert_eq!(events[0].result, AuditResult::Failure);
    }

    #[tokio::test]
    async fn test_default_log_config_change_and_degradation_event() {
        let logger = logger();
        logger
            .log_config_change(
                None,
                "update".to_string(),
                "rate_limit".to_string(),
                serde_json::json!({"enabled": true}),
            )
            .await;
        logger
            .log_degradation_event(
                Some("ws".to_string()),
                "degrade".to_string(),
                "segment".to_string(),
                "Normal".to_string(),
                "Critical".to_string(),
                serde_json::json!({}),
            )
            .await;
        logger
            .log_degradation_event(
                None,
                "recover".to_string(),
                "segment".to_string(),
                "Critical".to_string(),
                "Normal".to_string(),
                serde_json::json!({}),
            )
            .await;
        let events = logger.events.lock().expect("lock");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].result, AuditResult::Success);
        assert_eq!(events[1].result, AuditResult::Failure);
        assert_eq!(events[2].result, AuditResult::Success);
    }

    /// 新增审计事件类型与时钟 resolutions 的 JSON roundtrip 必须无损：
    /// server 审计链以 JSON 落盘，新值只增不改，既有事件不受影响（R-audit-201）。
    #[test]
    fn test_new_audit_event_types_serialize_roundtrip() {
        for event_type in [
            AuditEventType::FallbackUsed,
            AuditEventType::ClockBackwards,
            AuditEventType::IdGeneration,
        ] {
            let json = serde_json::to_string(&event_type).expect("serialize");
            let back: AuditEventType = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, event_type);
        }
        for resolution in [ClockResolution::Waited, ClockResolution::Errored] {
            let json = serde_json::to_string(&resolution).expect("serialize");
            let back: ClockResolution = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(serde_json::to_string(&back).expect("reserialize"), json);
        }
    }

    #[tokio::test]
    async fn test_default_log_clock_backwards_records_resolution() {
        let recorded = logger();
        recorded
            .log_clock_backwards(
                Some("ws".to_string()),
                "snowflake".to_string(),
                serde_json::json!({"drift_ms": 12}),
                ClockResolution::Waited,
            )
            .await;
        recorded
            .log_clock_backwards(
                Some("ws".to_string()),
                "snowflake".to_string(),
                serde_json::json!({"drift_ms": 2000}),
                ClockResolution::Errored,
            )
            .await;
        let events = recorded.events.lock().expect("lock");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].result, AuditResult::Success);
        assert_eq!(events[1].result, AuditResult::Failure);
        assert_eq!(events[1].details.as_ref().unwrap()["resolution"], "Errored");
    }
}
