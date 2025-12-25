use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventType {
    IdGeneration,
    BatchGeneration,
    Authentication,
    ConfigChange,
    DegradationEvent,
    RateLimitExceeded,
    HealthCheck,
    MetricsAccess,
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
