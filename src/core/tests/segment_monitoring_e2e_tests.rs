// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

#![cfg(test)]

//! Segment 算法端到端测试：覆盖默认 loader 与健康上报的公开 API 路径。

use crate::core::algorithm::segment::SegmentAlgorithm;
use crate::core::algorithm::{GenerateContext, HealthStatus, IdAlgorithm};
use crate::core::types::IdFormat;

fn make_ctx() -> GenerateContext {
    GenerateContext {
        workspace_id: "ws-e2e".to_string(),
        group_id: "grp-e2e".to_string(),
        biz_tag: "tag-e2e".to_string(),
        format: IdFormat::Numeric,
        prefix: None,
        dc: 0,
    }
}

#[tokio::test]
async fn e2e_segment_generate_via_default_loader_returns_id() {
    // DefaultSegmentLoader: timestamp-based segment (start_id = ts*10000), > 0
    let algo = SegmentAlgorithm::new(0);
    let id = algo.generate(&make_ctx()).await.unwrap();
    assert!(id.as_u128() > 0, "generated ID should be positive");
}

#[tokio::test]
async fn e2e_segment_health_check_idle_is_healthy() {
    // 空闲（无活动缓冲）不是故障：号段在首个请求时按需分配。
    let algo = SegmentAlgorithm::new(0);
    assert_eq!(algo.health_check(), HealthStatus::Healthy);
}
