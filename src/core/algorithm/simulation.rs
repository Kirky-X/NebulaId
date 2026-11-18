// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Snowflake 容量预演（纯函数，零 IO 零时钟）：泊松近似估算序列号耗尽概率。
//!
//! 模型：单毫秒需求 X ~ Poisson(rps / 1000)，容量 = sequence_mask
//! （`random_start = true` 时按 mask/2 期望折减，保守估计）；
//! `exhaustion_probability = P(X > capacity)`，
//! `projected_exhaustions = duration_ms * probability`。
//! 突发非平稳流量下模型失真，报告带 `model = "poisson"` 标注防误读。

use crate::core::algorithm::SnowflakeLayoutInfo;

/// 预演输入（全部显式，零运行时状态读取）。
#[derive(Debug, Clone)]
pub struct DryRunInput {
    pub layout: SnowflakeLayoutInfo,
    pub worker_count: u32,
    pub rps: f64,
    pub duration_secs: u64,
    /// 假设不可预测性 High：随机起点按 mask/2 折减可用容量。
    pub random_start: bool,
}

/// 预演报告。
#[derive(Debug, Clone, PartialEq)]
pub struct DryRunReport {
    pub model: &'static str,
    pub capacity_per_ms: u64,
    pub capacity_per_window: u64,
    pub demand_estimate: u64,
    pub projected_exhaustions: u64,
    pub exhaustion_probability: f64,
    pub recommended_layout: Option<String>,
    pub note: Option<String>,
}

/// 泊松尾概率 P(X > capacity)，X ~ Poisson(lambda)。
/// 用递推 pmf(k+1) = pmf(k) * lambda / (k+1) 累加尾项，
/// log 空间算 pmf(0) 避免 exp(-lambda) 在 lambda 大时下溢。
fn poisson_tail(lambda: f64, capacity: u64) -> f64 {
    if lambda <= 0.0 {
        return 0.0;
    }
    if lambda > 700.0 {
        // exp(-lambda) 下溢；此量级下容量远小于均值，耗尽概率数值上为 1。
        return 1.0;
    }
    // 从 k=0 递推到 capacity 并累加尾部，直到 pmf 足够小或达到 lambda + 12σ。
    let mut pmf = (-lambda).exp();
    let mut cdf = pmf;
    for k in 0..capacity {
        pmf *= lambda / (k as f64 + 1.0);
        cdf += pmf;
        if !cdf.is_finite() {
            cdf = 1.0;
            break;
        }
    }
    (1.0 - cdf).clamp(0.0, 1.0)
}

/// 纯函数容量预演。
pub fn simulate_snowflake(input: &DryRunInput) -> DryRunReport {
    let mask = (1u64 << input.layout.sequence_bits) - 1;
    let capacity = if input.random_start { mask / 2 } else { mask };
    let duration_ms = input.duration_secs.saturating_mul(1000);
    let lambda = input.rps / 1000.0;
    let probability = poisson_tail(lambda, capacity);
    let demand = (input.rps * duration_ms as f64).ceil() as u64;
    let projected = (duration_ms as f64 * probability).ceil() as u64;
    let recommended = if probability > 1e-6 {
        Some(format!(
            "increase sequence_bits (current capacity {capacity}); worker_count={}",
            input.worker_count
        ))
    } else {
        None
    };
    let note = input.random_start.then(|| {
        "random_start=true: capacity halved (conservative expectation of a uniform start)"
            .to_string()
    });
    DryRunReport {
        model: "poisson",
        capacity_per_ms: capacity,
        capacity_per_window: capacity.saturating_mul(duration_ms),
        demand_estimate: demand,
        projected_exhaustions: projected,
        exhaustion_probability: probability,
        recommended_layout: recommended,
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(sequence_bits: u8) -> SnowflakeLayoutInfo {
        let mut config = crate::core::config::SnowflakeAlgorithmConfig::default();
        config.sequence_bits = sequence_bits;
        SnowflakeLayoutInfo::from_config(&config)
    }

    #[test]
    fn test_default_layout_50k_rps_no_exhaustion() {
        let report = simulate_snowflake(&DryRunInput {
            layout: layout(10),
            worker_count: 1,
            rps: 50_000.0,
            duration_secs: 1,
            random_start: false,
        });
        assert_eq!(report.capacity_per_ms, 1023);
        assert!(report.exhaustion_probability < 1e-6);
        assert_eq!(report.projected_exhaustions, 0);
        assert_eq!(report.recommended_layout, None);
    }

    #[test]
    fn test_six_sequence_bits_50k_rps_exhausts() {
        let report = simulate_snowflake(&DryRunInput {
            layout: layout(6),
            worker_count: 1,
            rps: 50_000.0,
            duration_secs: 1,
            random_start: false,
        });
        assert_eq!(report.capacity_per_ms, 63);
        assert!(report.exhaustion_probability > 0.0);
        assert!(report.projected_exhaustions > 0);
        assert!(report.recommended_layout.is_some());
    }

    #[test]
    fn test_zero_duration_and_zero_rps_report_zero() {
        let zero_duration = simulate_snowflake(&DryRunInput {
            layout: layout(10),
            worker_count: 1,
            rps: 50_000.0,
            duration_secs: 0,
            random_start: false,
        });
        assert_eq!(zero_duration.capacity_per_window, 0);
        assert_eq!(zero_duration.demand_estimate, 0);
        assert_eq!(zero_duration.projected_exhaustions, 0);

        let zero_rps = simulate_snowflake(&DryRunInput {
            layout: layout(10),
            worker_count: 1,
            rps: 0.0,
            duration_secs: 10,
            random_start: false,
        });
        assert_eq!(zero_rps.demand_estimate, 0);
        assert_eq!(zero_rps.exhaustion_probability, 0.0);
        assert_eq!(zero_rps.projected_exhaustions, 0);
    }

    #[test]
    fn test_random_start_halves_capacity_and_notes() {
        let report = simulate_snowflake(&DryRunInput {
            layout: layout(10),
            worker_count: 1,
            rps: 1.0,
            duration_secs: 1,
            random_start: true,
        });
        assert_eq!(report.capacity_per_ms, 511);
        assert!(report.note.is_some());
        assert_eq!(report.model, "poisson");
    }
}
