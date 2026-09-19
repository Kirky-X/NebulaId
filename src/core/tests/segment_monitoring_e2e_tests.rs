// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

#![cfg(test)]

//! Segment algorithm end-to-end tests covering the live public-API paths that
//! exercise the default loader and health reporting.

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
    }
}

#[tokio::test]
async fn e2e_segment_generate_via_default_loader_returns_id() {
    // DefaultSegmentLoader returns a timestamp-based segment (start_id = ts*10000), always > 0
    let algo = SegmentAlgorithm::new(0);
    let id = algo.generate(&make_ctx()).await.unwrap();
    assert!(id.as_u128() > 0, "generated ID should be positive");
}

#[tokio::test]
async fn e2e_segment_health_check_without_db_returns_degraded() {
    // No active buffer -> health_check returns Degraded("No active buffers")
    let algo = SegmentAlgorithm::new(0);
    let status = algo.health_check();
    match status {
        HealthStatus::Degraded(msg) => assert_eq!(msg, "No active buffers"),
        other => panic!("expected Degraded, got {:?}", other),
    }
}
