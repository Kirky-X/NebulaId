use crate::algorithm::traits::{GenerateContext, IdAlgorithm};
use crate::algorithm::SegmentAlgorithm;
use crate::config::Config;
use crate::types::{AlgorithmType, Id};
use std::sync::Arc;
use std::time::Duration;

async fn create_segment_algorithm() -> Arc<dyn IdAlgorithm> {
    let config = Config::default();
    let algorithm = crate::algorithm::AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await
        .expect("Failed to build algorithm");
    Arc::from(algorithm)
}

#[tokio::test]
async fn test_dynamic_step_adjustment_high_qps() {
    let algorithm = create_segment_algorithm().await;
    let segment = algorithm
        .as_ref()
        .as_any()
        .downcast_ref::<SegmentAlgorithm>()
        .expect("Failed to downcast to SegmentAlgorithm");

    for _ in 0..100 {
        let ctx = GenerateContext {
            workspace_id: "test-workspace".to_string(),
            group_id: "test-group".to_string(),
            biz_tag: "test-tag".to_string(),
            format: crate::types::IdFormat::Numeric,
            prefix: None,
        };
        let _ = algorithm.generate(&ctx).await;
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    let adjusted_step = segment.get_current_step();
    let qps = segment.get_current_qps();

    tracing::debug!(
        adjusted_step = adjusted_step,
        qps = qps,
        "High QPS test result"
    );
}

#[tokio::test]
async fn test_dynamic_step_adjustment_low_qps() {
    let algorithm = create_segment_algorithm().await;
    let segment = algorithm
        .as_ref()
        .as_any()
        .downcast_ref::<SegmentAlgorithm>()
        .expect("Failed to downcast to SegmentAlgorithm");

    tokio::time::sleep(Duration::from_millis(1500)).await;

    let step = segment.get_current_step();
    let qps = segment.get_current_qps();

    tracing::debug!(step = step, qps = qps, "Low QPS test result");
    assert!(step >= 1, "Step should be at least 1");
}

#[tokio::test]
async fn test_step_bounds_enforcement() {
    let algorithm = create_segment_algorithm().await;
    let segment = algorithm
        .as_ref()
        .as_any()
        .downcast_ref::<SegmentAlgorithm>()
        .expect("Failed to downcast to SegmentAlgorithm");

    let step = segment.get_current_step();
    let config = Config::default();

    assert!(
        step >= config.algorithm.segment.min_step,
        "Step {} should be >= min_step {}",
        step,
        config.algorithm.segment.min_step
    );
    assert!(
        step <= config.algorithm.segment.max_step,
        "Step {} should be <= max_step {}",
        step,
        config.algorithm.segment.max_step
    );
}

#[tokio::test]
async fn test_adaptive_step_calculation() {
    let algorithm = create_segment_algorithm().await;
    let segment = algorithm
        .as_ref()
        .as_any()
        .downcast_ref::<SegmentAlgorithm>()
        .expect("Failed to downcast to SegmentAlgorithm");

    let initial_qps = segment.get_current_qps();
    assert_eq!(initial_qps, 0);

    for _ in 0..50 {
        let ctx = GenerateContext {
            workspace_id: "test-workspace".to_string(),
            group_id: "test-group".to_string(),
            biz_tag: "test-tag".to_string(),
            format: crate::types::IdFormat::Numeric,
            prefix: None,
        };
        let _ = algorithm.generate(&ctx).await;
    }

    tokio::time::sleep(Duration::from_millis(150)).await;

    let qps_after_requests = segment.get_current_qps();
    assert!(
        qps_after_requests > 0,
        "QPS should be measured after requests"
    );

    tracing::debug!(
        qps = qps_after_requests,
        "Adaptive step calculation test result"
    );
}
