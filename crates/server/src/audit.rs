use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventType {
    IdGeneration,
    BatchGeneration,
    Authentication,
    ConfigChange,
    RateLimitExceeded,
    HealthCheck,
    MetricsAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditResult {
    Success,
    Failure,
    Partial,
}

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
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        Self {
            id: COUNTER.fetch_add(1, Ordering::SeqCst),
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

    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    pub fn with_error(mut self, error: String) -> Self {
        self.error_message = Some(error);
        self
    }
}

#[derive(Clone)]
pub struct AuditLogger {
    events: Arc<Mutex<VecDeque<AuditEvent>>>,
    max_events: usize,
    total_logged: Arc<AtomicU64>,
    total_errors: Arc<AtomicU64>,
}

impl AuditLogger {
    pub fn new(max_events: usize) -> Self {
        Self {
            events: Arc::new(Mutex::new(VecDeque::with_capacity(max_events + 1))),
            max_events,
            total_logged: Arc::new(AtomicU64::new(0)),
            total_errors: Arc::new(AtomicU64::new(0)),
        }
    }

    pub async fn log(&self, event: AuditEvent) {
        let mut events = self.events.lock().await;

        while events.len() >= self.max_events {
            events.pop_front();
        }

        events.push_back(event.clone());
        self.total_logged.fetch_add(1, Ordering::SeqCst);

        info!(
            event_id = event.id,
            event_type = ?event.event_type,
            workspace = ?event.workspace_id,
            action = event.action,
            resource = event.resource,
            result = ?event.result,
            "Audit event recorded"
        );
    }

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

        let events = logger.get_events_by_type(AuditEventType::IdGeneration).await;
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

        let auth_events = logger.get_events_by_type(AuditEventType::Authentication).await;
        assert_eq!(auth_events.len(), 2);
        assert_eq!(auth_events[0].result, AuditResult::Success);
        assert_eq!(auth_events[1].result, AuditResult::Failure);
        assert_eq!(auth_events[1].error_message, Some("Invalid API key".to_string()));
    }
}
