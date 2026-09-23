// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Algorithm configuration (segment / snowflake / uuid_v8).

use crate::core::types::AlgorithmType;
use serde::{Deserialize, Serialize};

/// Segment algorithm configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SegmentAlgorithmConfig {
    /// Base step size for ID allocation
    pub base_step: u64,
    /// Minimum step size
    pub min_step: u64,
    /// Maximum step size
    pub max_step: u64,
    /// Threshold for dynamic step adjustment
    pub switch_threshold: f64,
}

impl Default for SegmentAlgorithmConfig {
    fn default() -> Self {
        Self {
            base_step: 1000,
            min_step: 500,
            max_step: 100000,
            switch_threshold: 0.1,
        }
    }
}

/// 防猜测混淆配置（change: fix-audit-defects-r1）。
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct ObfuscationConfig {
    /// 部署混淆密钥：16 个 hex 字符 = u64。None 时 `IdFormat::Obfuscated`
    /// 渲染显性报错（fail-closed，不生成默认弱密钥）。同一部署所有实例
    /// 必须持同一密钥，否则编码结果互不一致。
    #[serde(default)]
    pub key_hex: Option<String>,
}

impl std::fmt::Debug for ObfuscationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 手写 Debug：密钥与 api_key_salt 同级敏感，必须 REDACTED。
        f.debug_struct("ObfuscationConfig")
            .field("key_hex", &"[REDACTED]")
            .finish()
    }
}

impl ObfuscationConfig {
    /// 解析密钥为 [`crate::core::types::obfuscation::IdObfuscator`]。
    /// 未配置返回 `None`；hex 非法返回 `None`（配置校验已前置拦截，
    /// 此处兜底不 panic）。
    pub fn obfuscator(&self) -> Option<crate::core::types::obfuscation::IdObfuscator> {
        let hex = self.key_hex.as_deref()?;
        let key = u64::from_str_radix(hex, 16).ok()?;
        Some(crate::core::types::obfuscation::IdObfuscator::new(key))
    }
}

/// Snowflake algorithm configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnowflakeAlgorithmConfig {
    /// Number of bits for datacenter ID
    pub datacenter_id_bits: u8,
    /// Number of bits for worker ID
    pub worker_id_bits: u8,
    /// Number of bits for sequence number
    pub sequence_bits: u8,
    /// Clock drift threshold (milliseconds)
    pub clock_drift_threshold_ms: u64,
    /// 重启时钟守卫：运行期把最后发号时间戳持久化到 `./data/`，启动时
    /// 检测跨重启时钟回拨并 fail-closed（默认开启）。关闭后回拨检测
    /// 只剩运行中阈值机制，跨重启重复 ID 风险自负。
    #[serde(default = "default_true")]
    pub state_guard: bool,
}

fn default_true() -> bool {
    true
}

impl SnowflakeAlgorithmConfig {
    pub fn datacenter_id_mask(&self) -> u64 {
        (1 << self.datacenter_id_bits) - 1
    }

    pub fn worker_id_mask(&self) -> u64 {
        (1 << self.worker_id_bits) - 1
    }

    pub fn sequence_mask(&self) -> u64 {
        (1 << self.sequence_bits) - 1
    }

    pub fn timestamp_bits(&self) -> u8 {
        64 - self.datacenter_id_bits - self.worker_id_bits - self.sequence_bits
    }
}

impl Default for SnowflakeAlgorithmConfig {
    fn default() -> Self {
        Self {
            datacenter_id_bits: 3,
            worker_id_bits: 8,
            sequence_bits: 10,
            clock_drift_threshold_ms: 1000,
            state_guard: true,
        }
    }
}

/// UUID v8 configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UuidV8Config {
    /// Enable/disable UUID v8 generation
    pub enabled: bool,
}

impl Default for UuidV8Config {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Algorithm configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AlgorithmConfig {
    /// Default algorithm type
    pub default: String,
    /// Segment algorithm settings
    pub segment: SegmentAlgorithmConfig,
    /// Snowflake algorithm settings
    pub snowflake: SnowflakeAlgorithmConfig,
    /// UUID v8 settings
    pub uuid_v8: UuidV8Config,
    /// 防猜测混淆设置
    #[serde(default)]
    pub obfuscation: ObfuscationConfig,
}

impl Default for AlgorithmConfig {
    fn default() -> Self {
        Self {
            default: "segment".to_string(),
            segment: SegmentAlgorithmConfig::default(),
            snowflake: SnowflakeAlgorithmConfig::default(),
            uuid_v8: UuidV8Config::default(),
            obfuscation: ObfuscationConfig::default(),
        }
    }
}

impl AlgorithmConfig {
    pub fn get_default_algorithm(&self) -> AlgorithmType {
        self.default.parse().unwrap_or(AlgorithmType::Segment)
    }
}
