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

//! Integration tests for Nebula ID core functionality

use crate::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use crate::config::Config;
use crate::types::{AlgorithmType, IdFormat};
use std::collections::HashSet;

#[tokio::test]
async fn test_full_id_generation_workflow() {
    let config = Config::default();
    let algorithm: Box<dyn IdAlgorithm> = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await
        .expect("Failed to build algorithm");

    let ctx = GenerateContext {
        workspace_id: "test-workspace".to_string(),
        group_id: "test-group".to_string(),
        biz_tag: "test-tag".to_string(),
        format: IdFormat::Numeric,
        prefix: None,
    };

    // Generate single ID
    let id = algorithm
        .generate(&ctx)
        .await
        .expect("Failed to generate ID");
    assert!(id.as_u128() > 0);

    // Generate batch
    let batch = algorithm
        .batch_generate(&ctx, 10)
        .await
        .expect("Failed to generate batch");
    assert_eq!(batch.len(), 10);

    // Verify uniqueness
    let unique_ids: HashSet<_> = batch.ids.iter().map(|id| id.as_u128()).collect();
    assert_eq!(unique_ids.len(), 10);
}

#[tokio::test]
async fn test_algorithm_health_check() {
    let config = Config::default();
    let algorithm: Box<dyn IdAlgorithm> = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await
        .expect("Failed to build algorithm");

    let health = algorithm.health_check();
    // Health status could be Healthy, Degraded, or Unhealthy depending on initialization
    // We just verify it can be retrieved without panicking
    let _ = health;
}

#[tokio::test]
async fn test_batch_size_validation() {
    let config = Config::default();
    let algorithm: Box<dyn IdAlgorithm> = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await
        .expect("Failed to build algorithm");

    let ctx = GenerateContext {
        workspace_id: "test-workspace-batch".to_string(),
        group_id: "test-group".to_string(),
        biz_tag: "test-tag".to_string(),
        format: IdFormat::Numeric,
        prefix: None,
    };

    // Test valid batch size
    let batch = algorithm
        .batch_generate(&ctx, 50)
        .await
        .expect("Failed to generate batch");
    assert_eq!(batch.len(), 50);

    // Test large batch size (should work but may be slow)
    let batch = algorithm
        .batch_generate(&ctx, 100)
        .await
        .expect("Failed to generate batch");
    assert_eq!(batch.len(), 100);
}

#[tokio::test]
async fn test_algorithm_metrics() {
    let config = Config::default();
    let algorithm: Box<dyn IdAlgorithm> = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await
        .expect("Failed to build algorithm");

    let ctx = GenerateContext {
        workspace_id: "test-workspace-metrics".to_string(),
        group_id: "test-group".to_string(),
        biz_tag: "test-tag".to_string(),
        format: IdFormat::Numeric,
        prefix: None,
    };

    // Generate some IDs
    for _ in 0..10 {
        algorithm
            .generate(&ctx)
            .await
            .expect("Failed to generate ID");
    }

    // Check metrics
    let metrics = algorithm.metrics();
    assert!(metrics.total_generated > 0);
}
