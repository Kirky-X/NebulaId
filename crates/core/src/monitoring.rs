use crate::types::{AlgorithmType, Result};
use arc_swap::ArcSwap;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

const DEFAULT_EVALUATION_INTERVAL_MS: u64 = 1000;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AlertError {
    #[error("Alert rule not found: {0}")]
    NotFound(String),

    #[error("Alert evaluation failed: {0}")]
    EvaluationFailed(String),

    #[error("Alert channel closed")]
    ChannelClosed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertSeverity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertStatus {
    Firing,
    Resolved,
    Pending,
}

impl Default for AlertStatus {
    fn default() -> Self {
        AlertStatus::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub rule_name: String,
    pub severity: AlertSeverity,
    pub status: AlertStatus,
    pub message: String,
    pub labels: HashMap<String, String>,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub ends_at: Option<chrono::DateTime<chrono::Utc>>,
    pub generator: String,
}

impl Alert {
    pub fn new(
        rule_name: String,
        severity: AlertSeverity,
        message: String,
        labels: HashMap<String, String>,
    ) -> Self {
        Self {
            rule_name,
            severity,
            status: AlertStatus::Pending,
            message,
            labels,
            starts_at: chrono::Utc::now(),
            ends_at: None,
            generator: "nebula-id".to_string(),
        }
    }

    pub fn fire(&mut self) {
        self.status = AlertStatus::Firing;
        self.starts_at = chrono::Utc::now();
    }

    pub fn resolve(&mut self) {
        self.status = AlertStatus::Resolved;
        self.ends_at = Some(chrono::Utc::now());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub name: String,
    pub expression: String,
    pub for_duration: u64,
    pub severity: AlertSeverity,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub enabled: bool,
}

impl Default for AlertRule {
    fn default() -> Self {
        Self {
            name: "default_rule".to_string(),
            expression: "true".to_string(),
            for_duration: 60,
            severity: AlertSeverity::Warning,
            labels: HashMap::new(),
            annotations: HashMap::new(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChannelType {
    Webhook,
    Email,
    Slack,
    PagerDuty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationChannel {
    pub name: String,
    pub channel_type: ChannelType,
    pub config: HashMap<String, String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertingConfig {
    pub enabled: bool,
    pub evaluation_interval_ms: u64,
    pub rules: Vec<AlertRule>,
    pub channels: Vec<NotificationChannel>,
}

impl Default for AlertingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            evaluation_interval_ms: DEFAULT_EVALUATION_INTERVAL_MS,
            rules: Vec::new(),
            channels: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct AlertState {
    pub last_fired: Option<Instant>,
    pub consecutive_promotions: u8,
    pub current_status: AlertStatus,
}

pub struct AlertManager {
    config: Arc<ArcSwap<AlertingConfig>>,
    states: Arc<ArcSwap<HashMap<String, AlertState>>>,
    alerts_tx: broadcast::Sender<Alert>,
    eval_tx: broadcast::Sender<()>,
    shutdown_tx: Arc<RwLock<Option<broadcast::Sender<()>>>>,
}

impl AlertManager {
    pub fn new(config: AlertingConfig) -> (Self, broadcast::Receiver<Alert>) {
        let (alerts_tx, alerts_rx) = broadcast::channel(100);
        let (eval_tx, _) = broadcast::channel(1);
        let shutdown_tx = Arc::new(RwLock::new(None));

        let states = Arc::new(ArcSwap::from_pointee(HashMap::new()));

        let config = Arc::new(ArcSwap::from_pointee(config));

        let manager = Self {
            config,
            states,
            alerts_tx,
            eval_tx,
            shutdown_tx,
        };

        (manager, alerts_rx)
    }

    pub async fn start(&self) {
        let eval_rx = self.eval_tx.subscribe();
        let config = Arc::clone(&self.config);
        let states = Arc::clone(&self.states);
        let alerts_tx = self.alerts_tx.clone();
        let shutdown_rx = {
            let shutdown_tx_guard = self.shutdown_tx.read().await;
            shutdown_tx_guard.as_ref().cloned()
        };

        info!("AlertManager started");
    }

    fn evaluate_rules(
        states: &Arc<ArcSwap<HashMap<String, AlertState>>>,
        config: &Arc<ArcSwap<AlertingConfig>>,
        alerts_tx: &broadcast::Sender<Alert>,
    ) {
        let config_guard = config.load();
        let states_guard = states.load();

        for rule in &config_guard.rules {
            if !rule.enabled {
                continue;
            }

            let should_fire = Self::check_expression(&rule.expression);

            let mut alert = Alert::new(
                rule.name.clone(),
                rule.severity.clone(),
                format!("Alert rule '{}' triggered: {}", rule.name, rule.expression),
                rule.labels.clone(),
            );

            if should_fire {
                alert.fire();
                if let Err(e) = alerts_tx.send(alert) {
                    error!("Failed to send alert: {}", e);
                }
            }
        }
    }

    fn check_expression(expression: &str) -> bool {
        match expression {
            e if e.starts_with("id_generation_failed") => true,
            e if e.starts_with("latency_ms > ") => {
                let threshold: Vec<&str> = e.split(" > ").collect();
                threshold.len() == 2 && threshold[1].parse::<u64>().unwrap_or(0) > 100
            }
            e if e.starts_with("buffer_miss_rate > ") => {
                let threshold: Vec<&str> = e.split(" > ").collect();
                threshold.len() == 2 && threshold[1].parse::<f64>().unwrap_or(0.0) > 0.1
            }
            e if e.starts_with("segment_exhausted") => true,
            e if e.starts_with("clock_backward") => true,
            e => {
                warn!("Unknown alert expression: {}", e);
                false
            }
        }
    }

    pub fn update_config(&self, config: AlertingConfig) {
        self.config.store(Arc::new(config));
    }

    pub fn get_alerts(&self) -> Vec<Alert> {
        Vec::new()
    }

    pub async fn shutdown(&self) {
        let shutdown_tx = self.shutdown_tx.read().await;
        if let Some(tx) = &shutdown_tx {
            let _ = tx.send(());
        }
        info!("AlertManager shutdown");
    }
}

pub struct AlertService {
    config: Arc<ArcSwap<AlertingConfig>>,
    states: Arc<ArcSwap<HashMap<String, AlertState>>>,
    alerts_tx: broadcast::Sender<Alert>,
    eval_tx: broadcast::Sender<()>,
    shutdown_tx: Arc<RwLock<Option<broadcast::Sender<()>>>>,
}

impl AlertService {
    pub fn new(config: AlertingConfig) -> (Self, broadcast::Receiver<Alert>) {
        let (alerts_tx, alerts_rx) = broadcast::channel(100);
        let (eval_tx, _) = broadcast::channel(1);
        let shutdown_tx = Arc::new(RwLock::new(None));

        let states = Arc::new(ArcSwap::from_pointee(HashMap::new()));

        let service = Self {
            config: Arc::new(ArcSwap::from_pointee(config)),
            states,
            alerts_tx,
            eval_tx,
            shutdown_tx,
        };

        (service, alerts_rx)
    }

    pub async fn start(&self) {
        info!("AlertService starting...");
        let _ = self.eval_tx.send(());
        info!("AlertService started");
    }

    fn evaluate_rules(
        states: &Arc<ArcSwap<HashMap<String, AlertState>>>,
        config: &Arc<ArcSwap<AlertingConfig>>,
        alerts_tx: &broadcast::Sender<Alert>,
    ) {
        let config_guard = config.load();
        let _states_guard = states.load();

        for rule in &config_guard.rules {
            if !rule.enabled {
                continue;
            }

            let should_fire = Self::check_expression(&rule.expression);

            let mut alert = Alert::new(
                rule.name.clone(),
                rule.severity.clone(),
                format!("Alert rule '{}' triggered: {}", rule.name, rule.expression),
                rule.labels.clone(),
            );

            if should_fire {
                alert.fire();
                if let Err(e) = alerts_tx.send(alert) {
                    error!("Failed to send alert: {}", e);
                }
            }
        }
    }

    fn check_expression(expression: &str) -> bool {
        match expression {
            e if e.starts_with("id_generation_failed") => true,
            e if e.starts_with("latency_ms > ") => {
                let parts: Vec<&str> = e.split(" > ").collect();
                if parts.len() == 2 {
                    let threshold = parts[1].parse::<u64>().unwrap_or(0);
                    threshold > 100
                } else {
                    false
                }
            }
            e if e.starts_with("buffer_miss_rate > ") => {
                let parts: Vec<&str> = e.split(" > ").collect();
                if parts.len() == 2 {
                    let threshold = parts[1].parse::<f64>().unwrap_or(0.0);
                    threshold > 0.1
                } else {
                    false
                }
            }
            e if e.starts_with("segment_exhausted") => true,
            e if e.starts_with("clock_backward") => true,
            e => {
                warn!("Unknown alert expression: {}", e);
                false
            }
        }
    }

    pub fn update_config(&self, config: AlertingConfig) {
        self.config.store(Arc::new(config));
    }

    pub fn get_alerts(&self) -> Vec<Alert> {
        Vec::new()
    }

    pub async fn shutdown(&self) {
        info!("AlertService shutting down...");
        info!("AlertService shutdown complete");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_creation() {
        let alert = Alert::new(
            "test_rule".to_string(),
            AlertSeverity::Warning,
            "Test alert message".to_string(),
            HashMap::new(),
        );

        assert_eq!(alert.rule_name, "test_rule");
        assert_eq!(alert.severity, AlertSeverity::Warning);
        assert_eq!(alert.status, AlertStatus::Pending);
    }

    #[test]
    fn test_alert_fire_and_resolve() {
        let mut alert = Alert::new(
            "test_rule".to_string(),
            AlertSeverity::Critical,
            "Test alert".to_string(),
            HashMap::new(),
        );

        alert.fire();
        assert_eq!(alert.status, AlertStatus::Firing);

        alert.resolve();
        assert_eq!(alert.status, AlertStatus::Resolved);
        assert!(alert.ends_at.is_some());
    }

    #[test]
    fn test_alert_rule_default() {
        let rule: AlertRule = AlertRule::default();
        assert_eq!(rule.name, "default_rule");
        assert!(rule.enabled);
    }

    #[test]
    fn test_expression_parsing() {
        assert!(AlertService::check_expression("id_generation_failed"));
        assert!(AlertService::check_expression("segment_exhausted"));
        assert!(AlertService::check_expression("clock_backward"));
        assert!(!AlertService::check_expression("unknown_expression"));
    }
}
