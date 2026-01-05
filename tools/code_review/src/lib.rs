// Copyright © 2026 Nebula ID
//
// Multi-Agent Code Review System
// This module provides comprehensive code review capabilities including:
// - Security vulnerability detection
// - Performance optimization analysis
// - Code quality assessment
// - Architecture consistency validation

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeReviewResult {
    pub agent_name: String,
    pub findings: Vec<Finding>,
    pub summary: ReviewSummary,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub category: String,
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
    pub suggestion: String,
    pub code_snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSummary {
    pub total_issues: usize,
    pub critical_count: usize,
    pub high_count: usize,
    pub medium_count: usize,
    pub low_count: usize,
    pub score: f64,
}

impl Default for ReviewSummary {
    fn default() -> Self {
        Self::new()
    }
}

impl ReviewSummary {
    pub fn new() -> Self {
        Self {
            total_issues: 0,
            critical_count: 0,
            high_count: 0,
            medium_count: 0,
            low_count: 0,
            score: 100.0,
        }
    }

    pub fn calculate_score(&mut self) {
        let deductions = self.critical_count as f64 * 20.0
            + self.high_count as f64 * 10.0
            + self.medium_count as f64 * 5.0
            + self.low_count as f64 * 2.0;
        self.score = (100.0 - deductions).max(0.0);
        self.total_issues =
            self.critical_count + self.high_count + self.medium_count + self.low_count;
    }
}

pub struct CodeReviewSystem {
    project_root: PathBuf,
    config: ReviewConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewConfig {
    pub enabled_agents: Vec<String>,
    pub exclude_paths: Vec<String>,
    pub severity_threshold: Severity,
    pub auto_fix_suggestions: bool,
    pub output_format: OutputFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutputFormat {
    Json,
    Text,
    Markdown,
    Html,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            enabled_agents: vec![
                "security".to_string(),
                "performance".to_string(),
                "quality".to_string(),
                "architecture".to_string(),
            ],
            exclude_paths: vec![
                "target/".to_string(),
                "proto/".to_string(),
                "*.pb.rs".to_string(),
            ],
            severity_threshold: Severity::Low,
            auto_fix_suggestions: true,
            output_format: OutputFormat::Text,
        }
    }
}

impl CodeReviewSystem {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            config: ReviewConfig::default(),
        }
    }

    pub async fn run_full_review(&self) -> Vec<CodeReviewResult> {
        let mut results = Vec::new();
        let config = self.config.clone();
        let project_root = self.project_root.clone();

        // Run all enabled agents in parallel
        let handles: Vec<_> = config
            .enabled_agents
            .iter()
            .map(|agent| {
                let project_root = project_root.clone();
                let agent = agent.clone();
                tokio::spawn(async move {
                    let system = CodeReviewSystem {
                        project_root,
                        config: ReviewConfig::default(),
                    };
                    system.run_agent(&agent).await
                })
            })
            .collect();

        for handle in handles {
            if let Ok(result) = handle.await {
                results.push(result);
            }
        }

        results
    }

    async fn run_agent(&self, agent_name: &str) -> CodeReviewResult {
        match agent_name {
            "security" => self.run_security_agent().await,
            "performance" => self.run_performance_agent().await,
            "quality" => self.run_quality_agent().await,
            "architecture" => self.run_architecture_agent().await,
            _ => CodeReviewResult {
                agent_name: agent_name.to_string(),
                findings: Vec::new(),
                summary: ReviewSummary::new(),
                timestamp: chrono::Utc::now(),
            },
        }
    }

    async fn run_security_agent(&self) -> CodeReviewResult {
        let mut findings = Vec::new();
        let mut summary = ReviewSummary::new();

        // Search for potential security issues
        let rust_files = self.find_rust_files().await;

        for file in rust_files {
            if let Ok(content) = fs::read_to_string(&file).await {
                // Check for SQL injection patterns
                self.check_sql_injection(&content, &file, &mut findings);

                // Check for hardcoded secrets
                self.check_hardcoded_secrets(&content, &file, &mut findings);

                // Check for unsafe code blocks
                self.check_unsafe_blocks(&content, &file, &mut findings);

                // Check for authentication issues
                self.check_auth_patterns(&content, &file, &mut findings);
            }
        }

        summary.calculate_score();

        CodeReviewResult {
            agent_name: "security".to_string(),
            findings,
            summary,
            timestamp: chrono::Utc::now(),
        }
    }

    async fn run_performance_agent(&self) -> CodeReviewResult {
        let mut findings = Vec::new();
        let mut summary = ReviewSummary::new();

        let rust_files = self.find_rust_files().await;

        for file in rust_files {
            if let Ok(content) = fs::read_to_string(&file).await {
                // Check for unnecessary clones
                self.check_unnecessary_clones(&content, &file, &mut findings);

                // Check for inefficient string operations
                self.check_string_operations(&content, &file, &mut findings);

                // Check for blocking operations in async context
                self.check_blocking_async(&content, &file, &mut findings);

                // Check for memory allocation patterns
                self.check_memory_allocation(&content, &file, &mut findings);
            }
        }

        summary.calculate_score();

        CodeReviewResult {
            agent_name: "performance".to_string(),
            findings,
            summary,
            timestamp: chrono::Utc::now(),
        }
    }

    async fn run_quality_agent(&self) -> CodeReviewResult {
        let mut findings = Vec::new();
        let mut summary = ReviewSummary::new();

        let rust_files = self.find_rust_files().await;

        for file in rust_files {
            if let Ok(content) = fs::read_to_string(&file).await {
                // Check for code duplication
                self.check_code_duplication(&content, &file, &mut findings);

                // Check for function complexity
                self.check_function_complexity(&content, &file, &mut findings);

                // Check for missing documentation
                self.check_documentation(&content, &file, &mut findings);

                // Check for naming conventions
                self.check_naming_conventions(&content, &file, &mut findings);
            }
        }

        summary.calculate_score();

        CodeReviewResult {
            agent_name: "quality".to_string(),
            findings,
            summary,
            timestamp: chrono::Utc::now(),
        }
    }

    async fn run_architecture_agent(&self) -> CodeReviewResult {
        let mut findings = Vec::new();
        let mut summary = ReviewSummary::new();

        let rust_files = self.find_rust_files().await;

        for file in rust_files {
            if let Ok(content) = fs::read_to_string(&file).await {
                // Check for SOLID principles violations
                self.check_solid_principles(&content, &file, &mut findings);

                // Check for proper error handling
                self.check_error_handling(&content, &file, &mut findings);

                // Check for dependency direction
                self.check_dependencies(&content, &file, &mut findings);

                // Check for circular dependencies
                self.check_circular_dependencies(&content, &file, &mut findings);
            }
        }

        summary.calculate_score();

        CodeReviewResult {
            agent_name: "architecture".to_string(),
            findings,
            summary,
            timestamp: chrono::Utc::now(),
        }
    }

    // Helper methods for security checks
    fn check_sql_injection(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        let patterns = vec![
            (
                r"format!.*\{.*\}.*format",
                "Potential SQL injection via format string",
            ),
            (
                r"format!.*\+.*sql",
                "Potential SQL injection via string concatenation",
            ),
        ];

        for (pattern, message) in patterns {
            if let Ok(regex) = regex::Regex::new(pattern) {
                for (line_num, line) in content.lines().enumerate() {
                    if regex.is_match(line) {
                        findings.push(Finding {
                            severity: Severity::High,
                            category: "security".to_string(),
                            file: file.to_string_lossy().to_string(),
                            line: Some(line_num as u32 + 1),
                            message: message.to_string(),
                            suggestion: "Use parameterized queries instead of string formatting"
                                .to_string(),
                            code_snippet: Some(line.trim().to_string()),
                        });
                    }
                }
            }
        }
    }

    fn check_hardcoded_secrets(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        let patterns = vec![
            (
                r#"password\s*=\s*["'][^"']+["']"#,
                "Hardcoded password detected",
            ),
            (
                r#"secret\s*=\s*["'][^"']+["']"#,
                "Hardcoded secret detected",
            ),
            (
                r#"api_key\s*=\s*["'][^"']+["']"#,
                "Hardcoded API key detected",
            ),
            (r#"token\s*=\s*["'][^"']+["']"#, "Hardcoded token detected"),
        ];

        // Skip lines with environment variable placeholders
        let env_var_pattern = regex::Regex::new(r"\$\{[^}]+\}").unwrap();

        for (pattern, message) in patterns {
            if let Ok(regex) = regex::Regex::new(pattern) {
                for (line_num, line) in content.lines().enumerate() {
                    // Skip lines that use environment variables
                    if env_var_pattern.is_match(line) {
                        continue;
                    }
                    if regex.is_match(line) {
                        findings.push(Finding {
                            severity: Severity::Critical,
                            category: "security".to_string(),
                            file: file.to_string_lossy().to_string(),
                            line: Some(line_num as u32 + 1),
                            message: message.to_string(),
                            suggestion: "Use environment variables or secure secret management"
                                .to_string(),
                            code_snippet: Some(line.trim().to_string()),
                        });
                    }
                }
            }
        }
    }

    fn check_unsafe_blocks(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        let unsafe_pattern = regex::Regex::new(r"\bunsafe\s*\{").unwrap();
        for (line_num, line) in content.lines().enumerate() {
            if unsafe_pattern.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Medium,
                    category: "security".to_string(),
                    file: file.to_string_lossy().to_string(),
                    line: Some(line_num as u32 + 1),
                    message: "Unsafe block detected".to_string(),
                    suggestion:
                        "Review and document why unsafe is necessary. Consider safer alternatives."
                            .to_string(),
                    code_snippet: Some(line.trim().to_string()),
                });
            }
        }
    }

    fn check_auth_patterns(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        // Check for missing authorization checks
        if content.contains("axum::Extension<ApiKeyRole>")
            && !content.contains("StatusCode::FORBIDDEN")
        {
            findings.push(Finding {
                severity: Severity::Medium,
                category: "security".to_string(),
                file: file.to_string_lossy().to_string(),
                line: None,
                message: "Potential missing authorization check".to_string(),
                suggestion: "Ensure all protected endpoints have proper authorization checks"
                    .to_string(),
                code_snippet: None,
            });
        }
    }

    // Helper methods for performance checks
    fn check_unnecessary_clones(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        let pattern = regex::Regex::new(r"\.\s*clone\(\)\s*\.\s*clone\(\)").unwrap();
        for (line_num, line) in content.lines().enumerate() {
            if pattern.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Medium,
                    category: "performance".to_string(),
                    file: file.to_string_lossy().to_string(),
                    line: Some(line_num as u32 + 1),
                    message: "Unnecessary double clone detected".to_string(),
                    suggestion: "Consider restructuring to avoid unnecessary clones".to_string(),
                    code_snippet: Some(line.trim().to_string()),
                });
            }
        }
    }

    fn check_string_operations(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        let patterns = vec![
            (
                r"to_string\(\)\.as_str\(\)",
                "Inefficient to_string followed by as_str",
            ),
            (
                r"format!.*as_str\(\)",
                "Inefficient format followed by as_str",
            ),
        ];

        for (pattern, message) in patterns {
            if let Ok(regex) = regex::Regex::new(pattern) {
                for (line_num, line) in content.lines().enumerate() {
                    if regex.is_match(line) {
                        findings.push(Finding {
                            severity: Severity::Low,
                            category: "performance".to_string(),
                            file: file.to_string_lossy().to_string(),
                            line: Some(line_num as u32 + 1),
                            message: message.to_string(),
                            suggestion: "Use &str instead of String where possible".to_string(),
                            code_snippet: Some(line.trim().to_string()),
                        });
                    }
                }
            }
        }
    }

    fn check_blocking_async(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        let patterns = vec![
            (r"std::thread::spawn", "Thread spawn in async context"),
            (
                r"std::time::Duration::from_secs",
                "Hardcoded sleep duration",
            ),
        ];

        for (pattern, message) in patterns {
            if let Ok(regex) = regex::Regex::new(pattern) {
                for (line_num, line) in content.lines().enumerate() {
                    if regex.is_match(line) {
                        findings.push(Finding {
                            severity: Severity::Medium,
                            category: "performance".to_string(),
                            file: file.to_string_lossy().to_string(),
                            line: Some(line_num as u32 + 1),
                            message: message.to_string(),
                            suggestion: "Use tokio::time::sleep for async contexts".to_string(),
                            code_snippet: Some(line.trim().to_string()),
                        });
                    }
                }
            }
        }
    }

    fn check_memory_allocation(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        let pattern = regex::Regex::new(r"Vec::with_capacity\(\d+\)").unwrap();
        for (line_num, line) in content.lines().enumerate() {
            if pattern.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Low,
                    category: "performance".to_string(),
                    file: file.to_string_lossy().to_string(),
                    line: Some(line_num as u32 + 1),
                    message: "Pre-allocated Vec detected".to_string(),
                    suggestion: "Ensure capacity is based on actual requirements, not estimates"
                        .to_string(),
                    code_snippet: Some(line.trim().to_string()),
                });
            }
        }
    }

    // Helper methods for quality checks
    fn check_code_duplication(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        // Simple check for repeated code blocks (3+ identical lines)
        let lines: Vec<&str> = content.lines().collect();
        let mut duplicates = std::collections::HashMap::new();

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.len() > 20 {
                duplicates
                    .entry(trimmed.to_string())
                    .or_insert(Vec::new())
                    .push(i);
            }
        }

        for (code, positions) in duplicates {
            if positions.len() >= 3 {
                findings.push(Finding {
                    severity: Severity::Medium,
                    category: "quality".to_string(),
                    file: file.to_string_lossy().to_string(),
                    line: Some(positions[0] as u32 + 1),
                    message: format!(
                        "Code duplication detected (appears {} times)",
                        positions.len()
                    ),
                    suggestion: "Extract duplicated code into a shared function".to_string(),
                    code_snippet: Some(code.chars().take(100).collect()),
                });
            }
        }
    }

    fn check_function_complexity(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        // Check for deeply nested code
        let nested_pattern = regex::Regex::new(r"\{\s*\{\s*\{").unwrap();
        for (line_num, line) in content.lines().enumerate() {
            if nested_pattern.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Low,
                    category: "quality".to_string(),
                    file: file.to_string_lossy().to_string(),
                    line: Some(line_num as u32 + 1),
                    message: "Deeply nested code detected".to_string(),
                    suggestion: "Consider extracting nested code into separate functions"
                        .to_string(),
                    code_snippet: Some(line.trim().to_string()),
                });
            }
        }
    }

    fn check_documentation(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        let pub_fn_pattern = regex::Regex::new(r"pub\s+(?:async\s+)?fn\s+\w+").unwrap();

        for (line_num, line) in content.lines().enumerate() {
            if pub_fn_pattern.is_match(line) {
                // Check if there's documentation before this function
                let preceding_lines = &content.lines().take(line_num).collect::<Vec<_>>();
                let has_docs = preceding_lines
                    .iter()
                    .rev()
                    .take(5)
                    .any(|l| l.trim_start().starts_with("///"));

                if !has_docs {
                    findings.push(Finding {
                        severity: Severity::Low,
                        category: "quality".to_string(),
                        file: file.to_string_lossy().to_string(),
                        line: Some(line_num as u32 + 1),
                        message: "Public function without documentation".to_string(),
                        suggestion: "Add documentation comments for public APIs".to_string(),
                        code_snippet: Some(line.trim().to_string()),
                    });
                }
            }
        }
    }

    fn check_naming_conventions(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        // Check for SCREAMING_SNAKE_CASE constants
        let const_pattern = regex::Regex::new(r"const\s+[a-z_][a-z0-9_]*\s*=").unwrap();
        for (line_num, line) in content.lines().enumerate() {
            if const_pattern.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Low,
                    category: "quality".to_string(),
                    file: file.to_string_lossy().to_string(),
                    line: Some(line_num as u32 + 1),
                    message: "Constant should use SCREAMING_SNAKE_CASE".to_string(),
                    suggestion: "Rename constant to use UPPERCASE_WITH_UNDERSCORES".to_string(),
                    code_snippet: Some(line.trim().to_string()),
                });
            }
        }
    }

    // Helper methods for architecture checks
    fn check_solid_principles(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        // Check for God modules (too many responsibilities)
        if content.lines().count() > 500 {
            findings.push(Finding {
                severity: Severity::Medium,
                category: "architecture".to_string(),
                file: file.to_string_lossy().to_string(),
                line: None,
                message: "Large file detected - potential God module".to_string(),
                suggestion: "Consider splitting into smaller, focused modules".to_string(),
                code_snippet: None,
            });
        }

        // Check for proper trait usage
        if content.contains("impl Clone") && !content.contains("derive(Clone)") {
            findings.push(Finding {
                severity: Severity::Low,
                category: "architecture".to_string(),
                file: file.to_string_lossy().to_string(),
                line: None,
                message: "Manual Clone implementation detected".to_string(),
                suggestion: "Consider using derive macro for Clone when possible".to_string(),
                code_snippet: None,
            });
        }
    }

    fn check_error_handling(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        // Check for unwrap usage
        let unwrap_pattern = regex::Regex::new(r"\.(unwrap|expect)\(").unwrap();
        let mut unwrap_count = 0;

        for line in content.lines() {
            if unwrap_pattern.is_match(line) {
                unwrap_count += 1;
            }
        }

        if unwrap_count > 3 {
            findings.push(Finding {
                severity: Severity::Medium,
                category: "architecture".to_string(),
                file: file.to_string_lossy().to_string(),
                line: None,
                message: format!("Multiple unwrap/expect calls detected ({})", unwrap_count),
                suggestion: "Consider proper error handling instead of unwrap".to_string(),
                code_snippet: None,
            });
        }
    }

    fn check_dependencies(&self, content: &str, file: &Path, findings: &mut Vec<Finding>) {
        // Check for proper module structure
        let use_pattern = regex::Regex::new(r"use\s+(?:crate|self|super)::").unwrap();
        if !use_pattern.is_match(content) {
            findings.push(Finding {
                severity: Severity::Low,
                category: "architecture".to_string(),
                file: file.to_string_lossy().to_string(),
                line: None,
                message: "File may have external dependencies without proper module imports"
                    .to_string(),
                suggestion: "Ensure proper use of crate/self/super for internal imports"
                    .to_string(),
                code_snippet: None,
            });
        }
    }

    fn check_circular_dependencies(
        &self,
        _content: &str,
        _file: &Path,
        _findings: &mut Vec<Finding>,
    ) {
        // This would require analyzing the entire module graph
        // For now, we'll skip this check as it requires more complex analysis
    }

    async fn find_rust_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut stack = vec![self.project_root.clone()];

        while let Some(dir) = stack.pop() {
            if let Ok(mut entries) = fs::read_dir(&dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    if path.is_dir() {
                        if !self.should_exclude(&path) {
                            stack.push(path);
                        }
                    } else if path.extension().is_some_and(|e| e == "rs")
                        && !self.should_exclude(&path)
                    {
                        files.push(path);
                    }
                }
            }
        }

        files
    }

    fn should_exclude(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        for exclude in &self.config.exclude_paths {
            if path_str.contains(exclude.trim_end_matches('/')) {
                return true;
            }
        }

        false
    }
}
