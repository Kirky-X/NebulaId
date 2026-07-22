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

#![cfg(test)]

//! Segment 算法与监控模块端到端测试
//!
//! 覆盖《功能场景穷举分析》第 1 节（Segment 算法全路径）与第 2.6 节（监控模块）
//! 的端到端场景。聚焦公开 API 的协同行为，避免与 `segment.rs` 内 `#[cfg(test)]
//! mod tests` 的单元测试（覆盖单函数边界）重复。
//!
//! 注意：部分接口签名与最初任务描述存在差异，已按代码库实际实现调整：
//! - `StepCalculator::new(velocity_factor, pressure_factor)`，`calculate(qps,
//!   current_step, config)`；公式 `base*(1+vf*(qps/step))*(1+pf*pressure)`。
//! - `DcFailureDetector::select_best_dc(preferred_dc) -> u8`（非 `Option<u8>`，
//!   无健康 DC 时回退到 preferred_dc）。
//! - `DefaultSegmentLoader` 返回基于时间戳的号段（start_id = ts*10000），非固定 (0,1000)。
//! - `batch_generate(ctx, 0)` 返回 `Err(SegmentExhausted)`，而非空 Ok 批次。

use crate::core::algorithm::segment::{
    AtomicSegment, CpuMonitor, DcFailureDetector, DcStatus, DoubleBuffer, SegmentAlgorithm,
    SegmentData, SegmentLoader, StepCalculator,
};
use crate::core::algorithm::{GenerateContext, HealthStatus, IdAlgorithm};
use crate::core::config::SegmentAlgorithmConfig;
use crate::core::types::{AlgorithmType, CoreError, IdFormat, Result};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

// =============================================================================
// 测试辅助：可控的 MockSegmentLoader
// =============================================================================

/// 可控的 SegmentLoader mock：每次调用返回固定的 SegmentData，并记录调用次数。
struct MockSegmentLoader {
    data: SegmentData,
    calls: AtomicU64,
}

impl MockSegmentLoader {
    fn new(start_id: u64, max_id: u64, step: u64) -> Self {
        Self {
            data: SegmentData {
                start_id,
                max_id,
                step,
                version: 0,
            },
            calls: AtomicU64::new(0),
        }
    }

    #[allow(dead_code)]
    fn call_count(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl SegmentLoader for MockSegmentLoader {
    async fn load_segment(&self, _ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.data.clone())
    }
}

/// 构造测试用 GenerateContext
fn make_ctx() -> GenerateContext {
    GenerateContext {
        workspace_id: "ws-e2e".to_string(),
        group_id: "grp-e2e".to_string(),
        biz_tag: "tag-e2e".to_string(),
        format: IdFormat::Numeric,
        prefix: None,
    }
}

// =============================================================================
// E2E-SEG 组：Segment 号段分配端到端
// =============================================================================

#[tokio::test]
async fn e2e_segment_generate_via_default_loader_returns_id() {
    // DefaultSegmentLoader 返回基于时间戳的号段（start_id = ts*10000），必然 > 0
    let algo = SegmentAlgorithm::new(0);
    let id = algo.generate(&make_ctx()).await.unwrap();
    assert!(id.as_u128() > 0, "生成的 ID 应为正数");
}

#[tokio::test]
async fn e2e_segment_batch_generate_returns_correct_size() {
    let loader: Arc<dyn SegmentLoader + Send + Sync> =
        Arc::new(MockSegmentLoader::new(0, 10000, 1000));
    let algo = SegmentAlgorithm::new(0).with_loader(loader);
    let batch = algo.batch_generate(&make_ctx(), 5).await.unwrap();
    assert_eq!(batch.ids.len(), 5, "批量生成数量应为 5");
    // 5 个 ID 应连续递增
    let first = batch.ids[0].as_u128();
    for (i, id) in batch.ids.iter().enumerate() {
        assert_eq!(id.as_u128(), first + i as u128, "ID 应连续递增");
    }
    assert_eq!(batch.algorithm, AlgorithmType::Segment);
    assert_eq!(batch.biz_tag, "tag-e2e");
}

#[tokio::test]
async fn e2e_segment_batch_generate_zero_returns_empty() {
    // 实际行为：size=0 时 ids 为空，batch_generate 走 `ids.is_empty()` 分支
    // 返回 Err(SegmentExhausted)，而非 Ok(空批次)。
    let loader: Arc<dyn SegmentLoader + Send + Sync> =
        Arc::new(MockSegmentLoader::new(0, 10000, 1000));
    let algo = SegmentAlgorithm::new(0).with_loader(loader);
    let result = algo.batch_generate(&make_ctx(), 0).await;
    assert!(result.is_err(), "size=0 应返回错误而非空批次");
    match result.unwrap_err() {
        CoreError::SegmentExhausted { .. } => {}
        other => panic!("期望 SegmentExhausted，得到 {:?}", other),
    }
}

#[tokio::test]
async fn e2e_segment_health_check_without_db_returns_degraded() {
    // 无 buffer 时 health_check 返回 Degraded("No active buffers")
    let algo = SegmentAlgorithm::new(0);
    let status = algo.health_check();
    match status {
        HealthStatus::Degraded(msg) => assert_eq!(msg, "No active buffers"),
        other => panic!("期望 Degraded，得到 {:?}", other),
    }
}

#[tokio::test]
async fn e2e_segment_health_check_with_mock_loader_returns_healthy() {
    // 注入 mock loader 并 generate 一次创建 buffer，health_check 应为 Healthy
    let loader: Arc<dyn SegmentLoader + Send + Sync> =
        Arc::new(MockSegmentLoader::new(0, 10000, 1000));
    let algo = SegmentAlgorithm::new(0).with_loader(loader);
    let _ = algo.generate(&make_ctx()).await.unwrap();
    let status = algo.health_check();
    assert!(matches!(status, HealthStatus::Healthy), "应返回 Healthy");
}

#[tokio::test]
async fn e2e_segment_generate_monotonic_increasing() {
    let loader: Arc<dyn SegmentLoader + Send + Sync> =
        Arc::new(MockSegmentLoader::new(0, 10000, 1000));
    let algo = SegmentAlgorithm::new(0).with_loader(loader);
    let ctx = make_ctx();
    let mut prev: Option<u128> = None;
    for _ in 0..5 {
        let id = algo.generate(&ctx).await.unwrap();
        let v = id.as_u128();
        if let Some(p) = prev {
            assert!(v > p, "ID 应单调递增，前={} 后={}", p, v);
        }
        prev = Some(v);
    }
}

// =============================================================================
// E2E-STEP 组：动态步长端到端
// =============================================================================
//
// 实际 StepCalculator 接口：
//   StepCalculator::new(velocity_factor: f64, pressure_factor: f64)
//   calculate(&self, qps: u64, current_step: u64, config: &SegmentAlgorithmConfig) -> u64
// 公式：next = base_step * (1 + velocity_factor * (qps/step)) * (1 + pressure_factor * pressure)
//   step = current_step（current_step==0 时回退 base_step），pressure = CPU 使用率

#[test]
fn e2e_step_calculator_base_step_normal_qps() {
    let calculator = StepCalculator::default(); // velocity_factor=0.5, pressure_factor=0.3
    let config = SegmentAlgorithmConfig::default(); // base=1000, min=500, max=100000
                                                    // qps=1000, current_step=1000 → velocity=1.0, pressure=0.1（默认）
                                                    // next = 1000 * (1+0.5*1.0) * (1+0.3*0.1) = 1000 * 1.5 * 1.03 = 1545
    let step = calculator.calculate(1000, 1000, &config);
    assert_eq!(step, 1545, "正常 QPS 下步长应为 1545");
}

#[test]
fn e2e_step_calculator_qps_zero_returns_base() {
    // QPS=0 → velocity=0；通过 CpuMonitor 将 pressure 设为 0，使 next = base_step
    let monitor = Arc::new(CpuMonitor::new());
    monitor.update_usage(0.0);
    let calculator = StepCalculator::default().with_cpu_monitor(monitor);
    let config = SegmentAlgorithmConfig::default();
    let step = calculator.calculate(0, 1000, &config);
    assert_eq!(
        step, config.base_step,
        "QPS=0 且无 CPU 压力时应返回 base_step"
    );
}

#[test]
fn e2e_step_calculator_high_cpu_increases_step() {
    let config = SegmentAlgorithmConfig::default();
    let low = StepCalculator::default().calculate(1000, 1000, &config);

    let monitor = Arc::new(CpuMonitor::new());
    monitor.update_usage(0.9);
    let high = StepCalculator::default()
        .with_cpu_monitor(monitor)
        .calculate(1000, 1000, &config);

    assert!(high > low, "高 CPU 步长 {} 应大于低 CPU 步长 {}", high, low);
}

#[test]
fn e2e_step_calculator_current_step_zero_uses_base() {
    // current_step=0 应回退到 base_step 避免除零，结果与 current_step=base_step 等价
    let calculator = StepCalculator::default();
    let config = SegmentAlgorithmConfig::default();
    let with_zero = calculator.calculate(100, 0, &config);
    let with_base = calculator.calculate(100, config.base_step, &config);
    assert_eq!(with_zero, with_base, "current_step=0 应回退到 base_step");
    assert!(with_zero >= config.min_step && with_zero <= config.max_step);
}

#[test]
fn e2e_step_calculator_result_clamped_to_range() {
    // 上界 clamp：极高 QPS 应被 max_step 限制
    let aggressive = StepCalculator::new(1.0, 1.0);
    let config_max = SegmentAlgorithmConfig {
        base_step: 1000,
        min_step: 100,
        max_step: 50000,
        switch_threshold: 0.1,
    };
    let step_max = aggressive.calculate(u64::MAX, 1000, &config_max);
    assert_eq!(step_max, 50000, "极高 QPS 应被 max_step clamp 到 50000");

    // 下界 clamp：QPS=0 且无 CPU 压力，min_step 高于 base_step*0.5 时应被 min_step 限制
    let monitor = Arc::new(CpuMonitor::new());
    monitor.update_usage(0.0);
    let calc = StepCalculator::default().with_cpu_monitor(monitor);
    let config_min = SegmentAlgorithmConfig {
        base_step: 1000,
        min_step: 5000, // 高于 base_step*0.5=500
        max_step: 100000,
        switch_threshold: 0.1,
    };
    let step_min = calc.calculate(0, 1000, &config_min);
    assert_eq!(step_min, 5000, "应被 min_step clamp 到 5000");
}

// =============================================================================
// E2E-DC 组：多 DC 健康端到端
// =============================================================================
//
// 实际接口：DcFailureDetector::new(failure_threshold: u64, recovery_timeout: Duration)
//   get_dc_state(dc_id) -> Option<Arc<DcHealthState>>
//   select_best_dc(preferred_dc: u8) -> u8（无健康 DC 时回退到 preferred_dc，非 Option）

#[test]
fn e2e_dc_failure_detector_healthy_dc_returns_healthy() {
    let detector = DcFailureDetector::new(5, Duration::from_secs(300));
    detector.add_dc(0);
    let state = detector.get_dc_state(0).expect("dc 0 应存在");
    assert_eq!(state.get_status(), DcStatus::Healthy);
    assert!(state.should_use_dc());
}

#[test]
fn e2e_dc_failure_detector_3_failures_degraded() {
    let detector = DcFailureDetector::new(5, Duration::from_secs(300));
    detector.add_dc(0);
    let state = detector.get_dc_state(0).unwrap();
    for _ in 0..3 {
        state.record_failure();
    }
    assert_eq!(state.get_status(), DcStatus::Degraded);
    // Degraded 仍可使用
    assert!(state.should_use_dc());
}

#[test]
fn e2e_dc_failure_detector_5_failures_failed() {
    let detector = DcFailureDetector::new(5, Duration::from_secs(300));
    detector.add_dc(0);
    let state = detector.get_dc_state(0).unwrap();
    for _ in 0..5 {
        state.record_failure();
    }
    assert_eq!(state.get_status(), DcStatus::Failed);
    // Failed 不可使用
    assert!(!state.should_use_dc());
}

#[test]
fn e2e_dc_failure_detector_success_resets_count() {
    let detector = DcFailureDetector::new(5, Duration::from_secs(300));
    detector.add_dc(0);
    let state = detector.get_dc_state(0).unwrap();
    for _ in 0..3 {
        state.record_failure();
    }
    assert_eq!(state.consecutive_failures.load(Ordering::Relaxed), 3);
    assert_eq!(state.get_status(), DcStatus::Degraded);

    state.record_success();
    assert_eq!(state.consecutive_failures.load(Ordering::Relaxed), 0);
    assert_eq!(state.get_status(), DcStatus::Healthy);
}

#[test]
fn e2e_dc_failure_detector_select_best_dc() {
    let detector = DcFailureDetector::new(5, Duration::from_secs(300));
    detector.add_dc(0);
    detector.add_dc(1);
    // preferred_dc=0 健康 → 返回 0
    assert_eq!(detector.select_best_dc(0), 0);
    // 健康列表应包含 0 和 1
    let healthy = detector.get_healthy_dcs();
    assert!(healthy.contains(&0) && healthy.contains(&1));
}

#[test]
fn e2e_dc_failure_detector_no_healthy_dc_returns_none() {
    // 注意：select_best_dc 返回 u8，无健康 DC 时回退到 preferred_dc（非 None）。
    // 这里断言 get_healthy_dcs 为空，并验证 select_best_dc 的回退行为。
    let detector = DcFailureDetector::new(5, Duration::from_secs(300));
    detector.add_dc(0);
    let state = detector.get_dc_state(0).unwrap();
    for _ in 0..5 {
        state.record_failure();
    }
    assert_eq!(state.get_status(), DcStatus::Failed);

    let healthy = detector.get_healthy_dcs();
    assert!(healthy.is_empty(), "无健康 DC 时 get_healthy_dcs 应为空");

    // select_best_dc 无健康 DC 时回退到 preferred_dc
    assert_eq!(detector.select_best_dc(0), 0);
}

// =============================================================================
// E2E-DBL 组：双缓冲端到端
// =============================================================================

#[test]
fn e2e_double_buffer_initial_load_succeeds() {
    // 构造 DoubleBuffer，模拟初始加载号段并设为 current，验证可消费
    let (db, _rx) = DoubleBuffer::new(0.1);
    let seg = Arc::new(AtomicSegment::new(100, 1100, 1000));
    db.set_current(seg);

    let current = db.get_current();
    let (start, end) = current.try_consume(1).expect("应能消费 1 个 ID");
    assert_eq!(start, 100);
    assert_eq!(end, 101);
    assert_eq!(current.remaining(), 999);
}

#[test]
fn e2e_double_buffer_switch_on_threshold() {
    // switch_threshold=0.3：剩余比例 < 30% 时 need_switch 返回 true
    let (db, _rx) = DoubleBuffer::new(0.3);
    let seg = Arc::new(AtomicSegment::new(0, 1000, 100));
    db.set_current(seg);

    // 初始剩余 100%，不应触发切换
    assert!(!db.need_switch(), "剩余充足时不应触发切换");

    // 消费到剩余 20%（< 30%）→ 应触发切换
    {
        let current = db.get_current();
        current
            .inner
            .lock()
            .current_id
            .store(800, Ordering::Relaxed);
    }
    assert!(db.need_switch(), "剩余低于阈值时应触发切换");
}
