use crate::types::GlobalMetrics;
use arc_swap::ArcSwap;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::{
    broadcast,
    mpsc::{self, Receiver},
};
use tracing::{debug, error, info, warn};

const DEFAULT_EVALUATION_INTERVAL_MS: u64 = 1000;
const DEFAULT_FOR_DURATION_SECS: u64 = 60;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AlertError {
    #[error("Alert rule not found: {0}")]
    NotFound(String),

    #[error("Alert evaluation failed: {0}")]
    EvaluationFailed(String),

    #[error("Alert channel closed")]
    ChannelClosed,

    #[error("Notification failed: {0}")]
    NotificationFailed(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertSeverity {
    Critical = 1,
    Warning = 2,
    Info = 3,
}

impl std::fmt::Display for AlertSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertSeverity::Critical => write!(f, "Critical"),
            AlertSeverity::Warning => write!(f, "Warning"),
            AlertSeverity::Info => write!(f, "Info"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
    pub current_value: Option<String>,
}

impl Alert {
    pub fn new(
        rule_name: String,
        severity: AlertSeverity,
        message: String,
        labels: HashMap<String, String>,
        current_value: Option<String>,
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
            current_value,
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

    pub fn is_firing(&self) -> bool {
        self.status == AlertStatus::Firing
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
    pub description: String,
}

impl Default for AlertRule {
    fn default() -> Self {
        Self {
            name: "default_rule".to_string(),
            expression: "true".to_string(),
            for_duration: DEFAULT_FOR_DURATION_SECS,
            severity: AlertSeverity::Warning,
            labels: HashMap::new(),
            annotations: HashMap::new(),
            enabled: true,
            description: "Default alert rule".to_string(),
        }
    }
}

impl AlertRule {
    pub fn new<S: Into<String>>(name: S, expression: S, severity: AlertSeverity) -> Self {
        Self {
            name: name.into(),
            expression: expression.into(),
            severity,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChannelType {
    Webhook,
    Email,
    Slack,
    PagerDuty,
    Log,
    Stdout,
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelType::Webhook => write!(f, "Webhook"),
            ChannelType::Email => write!(f, "Email"),
            ChannelType::Slack => write!(f, "Slack"),
            ChannelType::PagerDuty => write!(f, "PagerDuty"),
            ChannelType::Log => write!(f, "Log"),
            ChannelType::Stdout => write!(f, "Stdout"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationChannel {
    pub name: String,
    pub channel_type: ChannelType,
    pub config: HashMap<String, String>,
    pub enabled: bool,
}

impl Default for NotificationChannel {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            channel_type: ChannelType::Log,
            config: HashMap::new(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertingConfig {
    pub enabled: bool,
    pub evaluation_interval_ms: u64,
    pub rules: Vec<AlertRule>,
    pub channels: Vec<NotificationChannel>,
    pub global_labels: HashMap<String, String>,
}

impl Default for AlertingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            evaluation_interval_ms: DEFAULT_EVALUATION_INTERVAL_MS,
            rules: Vec::new(),
            channels: Vec::new(),
            global_labels: HashMap::new(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct AlertState {
    pub last_fired: Option<Instant>,
    pub consecutive_promotions: u8,
    pub current_status: AlertStatus,
    pub pending_since: Option<Instant>,
    pub current_value: Option<String>,
}

impl AlertState {
    pub fn new() -> Self {
        Self {
            last_fired: None,
            consecutive_promotions: 0,
            current_status: AlertStatus::Pending,
            pending_since: None,
            current_value: None,
        }
    }

    pub fn should_fire(&self, for_duration: u64) -> bool {
        if let Some(pending_since) = self.pending_since {
            let elapsed = pending_since.elapsed();
            if elapsed.as_secs() >= for_duration {
                return self.consecutive_promotions > 0;
            }
        }
        false
    }

    pub fn promote(&mut self, value: Option<String>) {
        self.consecutive_promotions = self.consecutive_promotions.saturating_add(1);
        self.current_value = value;
        if self.current_status != AlertStatus::Firing {
            self.pending_since.get_or_insert(Instant::now());
        }
    }

    pub fn demote(&mut self) {
        self.consecutive_promotions = self.consecutive_promotions.saturating_sub(1);
        if self.consecutive_promotions == 0 {
            self.pending_since = None;
        }
    }

    pub fn reset(&mut self) {
        self.consecutive_promotions = 0;
        self.pending_since = None;
        self.current_status = AlertStatus::Pending;
        self.current_value = None;
    }
}

pub trait AlertEvaluator: Send + Sync {
    fn evaluate(&self, rule: &AlertRule, metrics: &GlobalMetrics) -> (bool, Option<String>);
}

pub struct DefaultEvaluator;

impl DefaultEvaluator {
    fn parse_threshold<T: std::str::FromStr>(
        expression: &str,
        metric_name: &str,
    ) -> Option<(T, T)> {
        let pattern = format!("{} {} ", metric_name, "{}");
        if let Some(rest) = expression.strip_prefix(&pattern) {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() == 2 {
                let threshold = parts[1].parse::<T>().ok()?;
                let value = parts[0].parse::<T>().ok()?;
                return Some((value, threshold));
            }
        }
        None
    }
}

impl AlertEvaluator for DefaultEvaluator {
    fn evaluate(&self, rule: &AlertRule, metrics: &GlobalMetrics) -> (bool, Option<String>) {
        let expr = rule.expression.trim();

        match expr {
            e if e.starts_with("id_generation_failed") => {
                let failed = metrics
                    .total_errors
                    .load(std::sync::atomic::Ordering::Relaxed);
                let firing = failed > 0;
                (firing, Some(failed.to_string()))
            }

            e if e.starts_with("id_generation_qps") => {
                if let Some((_value, threshold)) =
                    Self::parse_threshold::<u64>(e, "id_generation_qps")
                {
                    let qps = metrics
                        .total_requests
                        .load(std::sync::atomic::Ordering::Relaxed);
                    let firing = qps >= threshold;
                    (firing, Some(format!("current_qps: {}", qps)))
                } else {
                    (false, None)
                }
            }

            e if e.starts_with("latency_ms > ") => {
                if let Some((_, threshold)) = Self::parse_threshold::<u64>(e, "latency_ms") {
                    let snapshots = metrics.get_all_snapshots();
                    let max_latency = snapshots
                        .iter()
                        .map(|s| s.p99_latency_ms as u64)
                        .max()
                        .unwrap_or(0);
                    let firing = max_latency > threshold;
                    (firing, Some(format!("p99_latency_ms: {}", max_latency)))
                } else {
                    (false, None)
                }
            }

            e if e.starts_with("latency_p99 > ") => {
                if let Some((_, threshold)) = Self::parse_threshold::<f64>(e, "latency_p99") {
                    let snapshots = metrics.get_all_snapshots();
                    let max_latency = snapshots
                        .iter()
                        .map(|s| s.p99_latency_ms)
                        .max_by(|a, b| a.partial_cmp(&b).unwrap())
                        .unwrap_or(0.0);
                    let firing = max_latency > threshold;
                    (firing, Some(format!("p99_latency_ms: {:.2}", max_latency)))
                } else {
                    (false, None)
                }
            }

            e if e.starts_with("cache_hit_rate < ") => {
                if let Some((_, threshold)) = Self::parse_threshold::<f64>(e, "cache_hit_rate") {
                    let snapshots = metrics.get_all_snapshots();
                    let min_hit_rate = snapshots
                        .iter()
                        .map(|s| s.cache_hit_rate)
                        .min_by(|a, b| a.partial_cmp(&b).unwrap())
                        .unwrap_or(100.0);
                    let firing = min_hit_rate < threshold;
                    (firing, Some(format!("hit_rate: {:.2}%", min_hit_rate)))
                } else {
                    (false, None)
                }
            }

            e if e.starts_with("segment_exhausted") => {
                (true, Some("segment_buffer_exhausted".to_string()))
            }

            e if e.starts_with("clock_backward") => {
                for entry in metrics.algorithms.iter() {
                    let alg_metrics = entry.value();
                    if alg_metrics.p999_latency_ns.load(Ordering::Relaxed) > 0 {
                        return (true, Some("clock_backward_detected".to_string()));
                    }
                }
                (false, None)
            }

            e if e.starts_with("active_connections > ") => {
                if let Some((_, threshold)) = Self::parse_threshold::<u32>(e, "active_connections")
                {
                    let connections = metrics.active_connections.load(Ordering::Relaxed);
                    let firing = connections > threshold;
                    (firing, Some(format!("connections: {}", connections)))
                } else {
                    (false, None)
                }
            }

            e if e.starts_with("error_rate > ") => {
                if let Some((_, threshold)) = Self::parse_threshold::<f64>(e, "error_rate") {
                    let total = metrics.total_requests.load(Ordering::Relaxed);
                    let errors = metrics.total_errors.load(Ordering::Relaxed);
                    let rate = if total > 0 {
                        (errors as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };
                    let firing = rate > threshold;
                    (firing, Some(format!("error_rate: {:.2}%", rate)))
                } else {
                    (false, None)
                }
            }

            e => {
                warn!("Unknown alert expression: {}", e);
                (false, None)
            }
        }
    }
}

pub struct AlertNotificationSender {
    channels: Arc<ArcSwap<Vec<NotificationChannel>>>,
}

impl AlertNotificationSender {
    pub fn new(channels: Vec<NotificationChannel>) -> Self {
        Self {
            channels: Arc::new(ArcSwap::from_pointee(channels)),
        }
    }

    pub async fn send(&self, alert: &Alert) {
        let channels_guard = self.channels.load();
        for channel in channels_guard.iter() {
            if !channel.enabled {
                continue;
            }
            self.send_to_channel(alert, channel).await;
        }
    }

    async fn send_to_channel(&self, alert: &Alert, channel: &NotificationChannel) {
        match channel.channel_type {
            ChannelType::Stdout => {
                let level = match alert.severity {
                    AlertSeverity::Critical => tracing::Level::ERROR,
                    AlertSeverity::Warning => tracing::Level::WARN,
                    AlertSeverity::Info => tracing::Level::INFO,
                };
                match level {
                    tracing::Level::ERROR => {
                        error!(target: "alerts", "{}: {} - {}", alert.rule_name, alert.severity, alert.message)
                    }
                    tracing::Level::WARN => {
                        warn!(target: "alerts", "{}: {} - {}", alert.rule_name, alert.severity, alert.message)
                    }
                    tracing::Level::INFO => {
                        info!(target: "alerts", "{}: {} - {}", alert.rule_name, alert.severity, alert.message)
                    }
                    _ => {
                        debug!(target: "alerts", "{}: {} - {}", alert.rule_name, alert.severity, alert.message)
                    }
                }
            }

            ChannelType::Log => {
                info!(target: "alerts", "Alert: {:?} - {:?}", alert.rule_name, alert.message);
            }

            ChannelType::Webhook => {
                if let Some(url) = channel.config.get("url") {
                    let payload = serde_json::json!({
                        "rule_name": alert.rule_name,
                        "severity": format!("{:?}", alert.severity),
                        "message": alert.message,
                        "status": format!("{:?}", alert.status),
                        "labels": alert.labels,
                        "current_value": alert.current_value,
                        "starts_at": alert.starts_at.to_rfc3339(),
                    });

                    Self::send_webhook(url, &payload).await;
                }
            }

            ChannelType::Slack => {
                if let Some(webhook_url) = channel.config.get("webhook_url") {
                    let payload = serde_json::json!({
                        "text": format!("[{}] {}: {}", alert.severity, alert.rule_name, alert.message),
                        "attachments": [{
                            "color": match alert.severity {
                                AlertSeverity::Critical => "danger",
                                AlertSeverity::Warning => "warning",
                                AlertSeverity::Info => "good",
                            },
                            "fields": [
                                {"title": "Rule", "value": alert.rule_name, "short": true},
                                {"title": "Status", "value": format!("{:?}", alert.status), "short": true},
                            ]
                        }]
                    });

                    Self::send_webhook(webhook_url, &payload).await;
                }
            }

            ChannelType::Email | ChannelType::PagerDuty => {
                info!(
                    target: "alerts",
                    "Would send {} notification for alert: {}",
                    channel.channel_type, alert.rule_name
                );
            }
        }
    }

    async fn send_webhook(url: &str, payload: &serde_json::Value) {
        let client = reqwest::Client::new();
        let response = client.post(url).json(payload).send().await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                debug!("Webhook sent successfully to {}", url);
            }
            Ok(resp) => {
                error!("Webhook returned status: {}", resp.status());
            }
            Err(e) => {
                error!("Failed to send webhook to {}: {}", url, e);
            }
        }
    }

    pub fn update_channels(&self, channels: Vec<NotificationChannel>) {
        self.channels.store(Arc::new(channels));
    }
}

pub struct AlertManager {
    config: Arc<ArcSwap<AlertingConfig>>,
    states: Arc<DashMap<String, AlertState>>,
    alerts_tx: broadcast::Sender<Alert>,
    notification_sender: Arc<AlertNotificationSender>,
    metrics: Arc<GlobalMetrics>,
    evaluator: Arc<dyn AlertEvaluator>,
    running: Arc<AtomicBool>,
    eval_interval: Duration,
    shutdown_rx: Receiver<()>,
    alert_history: Arc<RwLock<Vec<Alert>>>,
    max_history_size: usize,
}

impl AlertManager {
    pub fn new(
        config: AlertingConfig,
        metrics: Arc<GlobalMetrics>,
        notification_sender: Arc<AlertNotificationSender>,
    ) -> (Self, broadcast::Receiver<Alert>) {
        let (alerts_tx, alerts_rx) = broadcast::channel(100);
        let (_, shutdown_rx) = mpsc::channel(1);

        let config_arc = Arc::new(ArcSwap::from_pointee(config));
        let states = Arc::new(DashMap::new());

        for rule in config_arc.load().rules.iter() {
            states.insert(rule.name.clone(), AlertState::new());
        }

        let manager = Self {
            config: config_arc,
            states,
            alerts_tx,
            notification_sender,
            metrics,
            evaluator: Arc::new(DefaultEvaluator),
            running: Arc::new(AtomicBool::new(false)),
            eval_interval: Duration::from_millis(DEFAULT_EVALUATION_INTERVAL_MS),
            shutdown_rx,
            alert_history: Arc::new(RwLock::new(Vec::new())),
            max_history_size: 1000,
        };

        (manager, alerts_rx)
    }

    pub async fn start(&mut self) {
        if self.running.swap(true, Ordering::SeqCst) {
            warn!("AlertManager is already running");
            return;
        }

        info!("AlertManager starting...");

        let config_guard = self.config.load();
        self.eval_interval = Duration::from_millis(config_guard.evaluation_interval_ms);

        self.run_evaluation_loop().await;

        info!("AlertManager started");
    }

    async fn run_evaluation_loop(&mut self) {
        let mut interval = tokio::time::interval(self.eval_interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.evaluate_all_rules().await;
                }
                _ = self.shutdown_rx.recv() => {
                    info!("AlertManager evaluation loop received shutdown signal");
                    break;
                }
            }
        }
    }

    async fn evaluate_all_rules(&self) {
        let config_guard = self.config.load();

        if !config_guard.enabled {
            return;
        }

        for rule in &config_guard.rules {
            if !rule.enabled {
                continue;
            }

            self.evaluate_rule(rule).await;
        }
    }

    async fn evaluate_rule(&self, rule: &AlertRule) {
        let config_guard = self.config.load();
        let (should_fire, current_value) = self.evaluator.evaluate(rule, &self.metrics);

        let mut state = self
            .states
            .entry(rule.name.clone())
            .or_insert_with(AlertState::new);

        if should_fire {
            state.promote(current_value.clone());

            if state.should_fire(rule.for_duration) {
                let mut alert = Alert::new(
                    rule.name.clone(),
                    rule.severity,
                    self.format_message(rule, current_value.as_deref()),
                    self.merge_labels(&rule.labels, &config_guard),
                    current_value.clone(),
                );
                alert.fire();

                if state.current_status != AlertStatus::Firing {
                    state.current_status = AlertStatus::Firing;
                    state.last_fired = Some(Instant::now());

                    self.store_alert_to_history(&alert);

                    if let Err(e) = self.alerts_tx.send(alert.clone()) {
                        error!("Failed to send alert: {}", e);
                    }

                    self.notification_sender.send(&alert).await;
                }
            }
        } else {
            state.demote();

            if state.current_status == AlertStatus::Firing && state.consecutive_promotions == 0 {
                state.current_status = AlertStatus::Resolved;

                let mut alert = Alert::new(
                    rule.name.clone(),
                    rule.severity,
                    format!("Alert resolved: {}", rule.name),
                    self.merge_labels(&rule.labels, &config_guard),
                    None,
                );
                alert.resolve();
                self.store_alert_to_history(&alert);

                if let Err(e) = self.alerts_tx.send(alert.clone()) {
                    error!("Failed to send resolved alert: {}", e);
                }

                self.notification_sender.send(&alert).await;
            }
        }
    }

    fn format_message(&self, rule: &AlertRule, current_value: Option<&str>) -> String {
        if let Some(desc) = rule.annotations.get("summary") {
            if let Some(value) = current_value {
                return format!("{} (current: {})", desc, value);
            }
            return desc.clone();
        }
        format!("Alert rule '{}' triggered: {}", rule.name, rule.expression)
    }

    fn merge_labels(
        &self,
        rule_labels: &HashMap<String, String>,
        config: &AlertingConfig,
    ) -> HashMap<String, String> {
        let mut merged = config.global_labels.clone();
        for (k, v) in rule_labels {
            merged.insert(k.clone(), v.clone());
        }
        merged
    }

    fn store_alert_to_history(&self, alert: &Alert) {
        let mut history = self.alert_history.write().unwrap();
        if history.len() >= self.max_history_size {
            history.remove(0);
        }
        history.push(alert.clone());
    }

    pub fn update_config(&mut self, config: AlertingConfig) {
        self.eval_interval = Duration::from_millis(config.evaluation_interval_ms);
        self.config.store(Arc::new(config));
    }

    pub fn get_state(&self, rule_name: &str) -> Option<AlertState> {
        self.states.get(rule_name).map(|s| s.clone())
    }

    pub fn get_all_states(&self) -> Vec<(String, AlertState)> {
        self.states
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    pub fn get_alerts(&self) -> Vec<Alert> {
        let history = self.alert_history.read().unwrap();
        history.clone()
    }

    pub fn get_alerts_by_severity(&self, severity: AlertSeverity) -> Vec<Alert> {
        let history = self.alert_history.read().unwrap();
        history
            .iter()
            .filter(|a| a.severity == severity)
            .cloned()
            .collect()
    }

    pub fn get_alerts_by_status(&self, status: AlertStatus) -> Vec<Alert> {
        let history = self.alert_history.read().unwrap();
        history
            .iter()
            .filter(|a| a.status == status)
            .cloned()
            .collect()
    }

    pub fn get_firing_alerts(&self) -> Vec<Alert> {
        self.get_alerts_by_status(AlertStatus::Firing)
    }

    pub fn get_recent_alerts(&self, limit: usize) -> Vec<Alert> {
        let history = self.alert_history.read().unwrap();
        history.iter().rev().take(limit).cloned().collect()
    }

    pub fn get_alert_count(&self) -> usize {
        let history = self.alert_history.read().unwrap();
        history.len()
    }

    pub fn clear_alert_history(&self) {
        let mut history = self.alert_history.write().unwrap();
        history.clear();
    }

    pub fn shutdown(&mut self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }

        info!("AlertManager shutting down...");
        let _ = self.shutdown_rx.close();
        info!("AlertManager shutdown complete");
    }

    pub fn add_rule(&self, rule: AlertRule) {
        self.states
            .entry(rule.name.clone())
            .or_insert_with(AlertState::new);
        let config = self.config.load().as_ref().clone();
        let mut new_config = config;
        new_config.rules.push(rule);
        self.config.store(Arc::new(new_config));
    }

    pub fn remove_rule(&self, rule_name: &str) {
        let config = self.config.load().as_ref().clone();
        let mut new_config = config;
        new_config.rules.retain(|r| r.name != rule_name);
        self.config.store(Arc::new(new_config));
        self.states.remove(rule_name);
    }
}

pub fn default_alerting_config() -> AlertingConfig {
    AlertingConfig {
        enabled: true,
        evaluation_interval_ms: 1000,
        rules: vec![
            AlertRule {
                name: "high_latency".to_string(),
                expression: "latency_p99 > 100".to_string(),
                for_duration: 60,
                severity: AlertSeverity::Warning,
                labels: HashMap::new(),
                annotations: [(
                    "summary".to_string(),
                    "P99 latency exceeds 100ms".to_string(),
                )]
                .iter()
                .cloned()
                .collect(),
                enabled: true,
                description: "High P99 latency alert".to_string(),
            },
            AlertRule {
                name: "low_cache_hit_rate".to_string(),
                expression: "cache_hit_rate < 95".to_string(),
                for_duration: 120,
                severity: AlertSeverity::Warning,
                labels: HashMap::new(),
                annotations: [(
                    "summary".to_string(),
                    "Cache hit rate below 95%".to_string(),
                )]
                .iter()
                .cloned()
                .collect(),
                enabled: true,
                description: "Low cache hit rate alert".to_string(),
            },
            AlertRule {
                name: "generation_failures".to_string(),
                expression: "id_generation_failed".to_string(),
                for_duration: 10,
                severity: AlertSeverity::Critical,
                labels: HashMap::new(),
                annotations: [(
                    "summary".to_string(),
                    "ID generation failures detected".to_string(),
                )]
                .iter()
                .cloned()
                .collect(),
                enabled: true,
                description: "ID generation failure alert".to_string(),
            },
            AlertRule {
                name: "high_error_rate".to_string(),
                expression: "error_rate > 1".to_string(),
                for_duration: 60,
                severity: AlertSeverity::Critical,
                labels: HashMap::new(),
                annotations: [("summary".to_string(), "Error rate exceeds 1%".to_string())]
                    .iter()
                    .cloned()
                    .collect(),
                enabled: true,
                description: "High error rate alert".to_string(),
            },
        ],
        channels: vec![NotificationChannel {
            name: "stdout".to_string(),
            channel_type: ChannelType::Stdout,
            config: HashMap::new(),
            enabled: true,
        }],
        global_labels: [("service".to_string(), "nebula-id".to_string())]
            .iter()
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::GlobalMetrics;

    #[test]
    fn test_alert_creation() {
        let alert = Alert::new(
            "test_rule".to_string(),
            AlertSeverity::Warning,
            "Test alert message".to_string(),
            HashMap::new(),
            Some("value".to_string()),
        );

        assert_eq!(alert.rule_name, "test_rule");
        assert_eq!(alert.severity, AlertSeverity::Warning);
        assert_eq!(alert.status, AlertStatus::Pending);
        assert_eq!(alert.current_value, Some("value".to_string()));
    }

    #[test]
    fn test_alert_fire_and_resolve() {
        let mut alert = Alert::new(
            "test_rule".to_string(),
            AlertSeverity::Critical,
            "Test alert".to_string(),
            HashMap::new(),
            None,
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
    fn test_alert_state_promote_demote() {
        let mut state = AlertState::new();

        assert_eq!(state.consecutive_promotions, 0);

        state.promote(Some("value1".to_string()));
        assert_eq!(state.consecutive_promotions, 1);
        assert_eq!(state.current_value, Some("value1".to_string()));

        state.promote(Some("value2".to_string()));
        assert_eq!(state.consecutive_promotions, 2);

        state.demote();
        assert_eq!(state.consecutive_promotions, 1);

        state.reset();
        assert_eq!(state.consecutive_promotions, 0);
        assert_eq!(state.current_status, AlertStatus::Pending);
    }

    #[tokio::test]
    async fn test_alert_state_should_fire() {
        let mut state = AlertState::new();

        assert!(!state.should_fire(1));

        state.promote(None);
        assert!(!state.should_fire(1));

        for _ in 0..60 {
            state.promote(None);
        }

        assert!(state.consecutive_promotions >= 1);
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        assert!(state.should_fire(1));
    }

    #[tokio::test]
    async fn test_alert_manager_lifecycle() {
        let metrics = Arc::new(GlobalMetrics::new());
        let channels = vec![NotificationChannel {
            name: "test".to_string(),
            channel_type: ChannelType::Stdout,
            config: HashMap::new(),
            enabled: true,
        }];
        let sender = Arc::new(AlertNotificationSender::new(channels));

        let config = AlertingConfig {
            enabled: true,
            evaluation_interval_ms: 100,
            rules: vec![AlertRule::new(
                "test_rule",
                "id_generation_failed",
                AlertSeverity::Warning,
            )],
            channels: vec![],
            global_labels: HashMap::new(),
        };

        let (mut manager, _rx) = AlertManager::new(config, metrics.clone(), sender);

        manager.add_rule(AlertRule::new(
            "another_rule",
            "error_rate > 0",
            AlertSeverity::Critical,
        ));

        assert_eq!(manager.get_all_states().len(), 2);

        manager.remove_rule("test_rule");
        assert_eq!(manager.get_all_states().len(), 1);

        manager.shutdown();
    }

    #[test]
    fn test_expression_parsing() {
        let evaluator = DefaultEvaluator;
        let metrics = GlobalMetrics::new();

        let rule = AlertRule::new("qps_test", "id_generation_qps > 1000", AlertSeverity::Info);
        let result = evaluator.evaluate(&rule, &metrics);
        assert!(!result.0);

        let rule = AlertRule::new("latency_test", "latency_p99 > 100", AlertSeverity::Warning);
        let result = evaluator.evaluate(&rule, &metrics);
        assert!(!result.0);

        let rule = AlertRule::new("error_test", "error_rate > 1", AlertSeverity::Critical);
        let result = evaluator.evaluate(&rule, &metrics);
        assert!(!result.0);
    }
}
