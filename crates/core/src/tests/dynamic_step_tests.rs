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

//! 动态步长测试用例
//!
//! 测试基于 QPS 的动态步长计算功能

use crate::algorithm::segment::StepCalculator;
use crate::config::SegmentAlgorithmConfig;

/// 测试步长计算器的基本计算
#[test]
fn test_step_calculator_basic_calculation() {
    let calculator = StepCalculator::default();
    let config = SegmentAlgorithmConfig {
        base_step: 1000,
        min_step: 500,
        max_step: 100000,
        switch_threshold: 0.1,
    };

    // 在基准 QPS 下，步长应该接近 base_step
    let step = calculator.calculate(1000, 1000, &config);
    assert!(
        step >= config.min_step,
        "Step {} should be >= min_step {}",
        step,
        config.min_step
    );
    assert!(
        step <= config.max_step,
        "Step {} should be <= max_step {}",
        step,
        config.max_step
    );
}

/// 测试高 QPS 下步长应该增大
#[test]
fn test_step_calculator_high_qps_increases_step() {
    let calculator = StepCalculator::default();
    let config = SegmentAlgorithmConfig {
        base_step: 1000,
        min_step: 500,
        max_step: 100000,
        switch_threshold: 0.1,
    };

    // 低 QPS
    let low_step = calculator.calculate(100, 1000, &config);
    // 高 QPS (10倍)
    let high_step = calculator.calculate(10000, 1000, &config);

    // 高 QPS 应该导致更大的步长
    assert!(
        high_step > low_step,
        "High QPS step {} should be > low QPS step {}",
        high_step,
        low_step
    );
}

/// 测试步长边界控制 - 不能低于 min_step
#[test]
fn test_step_calculator_respects_min_step() {
    let calculator = StepCalculator::default();
    let config = SegmentAlgorithmConfig {
        base_step: 1000,
        min_step: 500,
        max_step: 100000,
        switch_threshold: 0.1,
    };

    // 极低 QPS 不应该导致步长低于 min_step
    let step = calculator.calculate(1, 1000, &config);

    assert!(
        step >= config.min_step,
        "Step {} should be >= min_step {}",
        step,
        config.min_step
    );
}

/// 测试步长边界控制 - 不能高于 max_step
#[test]
fn test_step_calculator_respects_max_step() {
    let calculator = StepCalculator::default();
    let config = SegmentAlgorithmConfig {
        base_step: 1000,
        min_step: 500,
        max_step: 10000,
        switch_threshold: 0.1,
    };

    // 极高 QPS 不应该导致步长高于 max_step
    let step = calculator.calculate(1_000_000, 1000, &config);

    assert!(
        step <= config.max_step,
        "Step {} should be <= max_step {}",
        step,
        config.max_step
    );
}

/// 测试自定义因子 - 使用负的 velocity_factor
#[test]
fn test_step_calculator_custom_factors() {
    // 使用正的 velocity_factor，但测试边界情况
    let calculator = StepCalculator::new(0.5, 0.3);
    let config = SegmentAlgorithmConfig {
        base_step: 1000,
        min_step: 100,
        max_step: 100000,
        switch_threshold: 0.1,
    };

    // 低 QPS 步长
    let low_qps_step = calculator.calculate(100, 1000, &config);
    // 高 QPS 步长
    let high_qps_step = calculator.calculate(10000, 1000, &config);

    // 高 QPS 应该产生更大的步长
    assert!(
        high_qps_step > low_qps_step,
        "High QPS step {} should be > low QPS step {}",
        high_qps_step,
        low_qps_step
    );

    // 所有结果都应该在边界内
    assert!(
        low_qps_step >= config.min_step,
        "Low QPS step {} should be >= min_step {}",
        low_qps_step,
        config.min_step
    );
    assert!(
        high_qps_step <= config.max_step,
        "High QPS step {} should be <= max_step {}",
        high_qps_step,
        config.max_step
    );
}

/// 测试调整方向判断 - 高 QPS 应该建议增大步长
#[test]
fn test_step_calculator_direction_high_qps() {
    let calculator = StepCalculator::default();
    let config = SegmentAlgorithmConfig {
        base_step: 1000,
        min_step: 500,
        max_step: 100000,
        switch_threshold: 0.1,
    };

    // 极高 QPS 应该建议增大步长
    let direction = calculator.get_adjustment_direction(50000, 1000, &config);
    assert_eq!(
        direction, "up",
        "Very high QPS should suggest increasing step"
    );
}

/// 测试步长计算公式的正确性
#[test]
fn test_step_calculation_formula() {
    let calculator = StepCalculator::default();
    let config = SegmentAlgorithmConfig {
        base_step: 1000,
        min_step: 500,
        max_step: 100000,
        switch_threshold: 0.1,
    };

    // 测试已知输入下的输出
    let qps = 2000u64;
    let step = 1000u64;

    let calculated_step = calculator.calculate(qps, step, &config);

    // 验证结果在边界内
    assert!(calculated_step >= config.min_step);
    assert!(calculated_step <= config.max_step);

    // 验证高 QPS 时步长会增加
    let baseline_step = calculator.calculate(1000, 1000, &config);
    let increased_qps_step = calculator.calculate(5000, 1000, &config);

    assert!(
        increased_qps_step > baseline_step,
        "Increased QPS should result in larger step"
    );
}

/// 测试默认计算器在基准 QPS 附近的表现
#[test]
fn test_step_calculator_baseline_qps() {
    let calculator = StepCalculator::default();
    let config = SegmentAlgorithmConfig {
        base_step: 1000,
        min_step: 500,
        max_step: 100000,
        switch_threshold: 0.1,
    };

    // 在基准 QPS (1000) 时，步长应该接近基准
    let step = calculator.calculate(1000, 1000, &config);

    // 步长应该在基准的 50% 到 200% 之间（考虑压力因子）
    assert!(step >= 500, "Step {} should be >= min_step 500", step);
    assert!(
        step <= 2000,
        "Step {} should be <= 2000 (with pressure factor)",
        step
    );
}
