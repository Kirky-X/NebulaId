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

use async_trait::async_trait;
use nebula_core::algorithm::{DegradationManager, HealthStatus, IdGenerator as CoreIdGenerator};
use nebula_core::types::AlgorithmType;
use nebula_core::{CoreError, Id, Result};
use std::sync::Arc;

pub struct MockIdGenerator {
    counter: Arc<std::sync::atomic::AtomicU64>,
    degradation_manager: Arc<DegradationManager>,
}

impl MockIdGenerator {
    pub fn new() -> Self {
        Self {
            counter: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            degradation_manager: Arc::new(DegradationManager::new(None, None)),
        }
    }
}

impl Default for MockIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CoreIdGenerator for MockIdGenerator {
    async fn generate(&self, workspace: &str, _group: &str, _biz_tag: &str) -> Result<Id> {
        if workspace.is_empty() {
            return Err(CoreError::InvalidInput(
                "workspace cannot be empty".to_string(),
            ));
        }
        let id = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Id::from_u128(id.into()))
    }

    async fn batch_generate(
        &self,
        workspace: &str,
        _group: &str,
        _biz_tag: &str,
        size: usize,
    ) -> Result<Vec<Id>> {
        if workspace.is_empty() {
            return Err(CoreError::InvalidInput(
                "workspace cannot be empty".to_string(),
            ));
        }
        let start = self
            .counter
            .fetch_add(size as u64, std::sync::atomic::Ordering::SeqCst);
        Ok((start + 1..=start + size as u64)
            .map(|v| Id::from_u128(v.into()))
            .collect())
    }

    async fn generate_with_algorithm(
        &self,
        _algorithm: AlgorithmType,
        workspace: &str,
        _group: &str,
        _biz_tag: &str,
    ) -> Result<Id> {
        if workspace.is_empty() {
            return Err(CoreError::InvalidInput(
                "workspace cannot be empty".to_string(),
            ));
        }
        let id = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Id::from_u128(id.into()))
    }

    async fn batch_generate_with_algorithm(
        &self,
        _algorithm: AlgorithmType,
        workspace: &str,
        _group: &str,
        _biz_tag: &str,
        size: usize,
    ) -> Result<Vec<Id>> {
        if workspace.is_empty() {
            return Err(CoreError::InvalidInput(
                "workspace cannot be empty".to_string(),
            ));
        }
        let start = self
            .counter
            .fetch_add(size as u64, std::sync::atomic::Ordering::SeqCst);
        Ok((start + 1..=start + size as u64)
            .map(|v| Id::from_u128(v.into()))
            .collect())
    }

    async fn get_algorithm_name(
        &self,
        _workspace: &str,
        _group: &str,
        _biz_tag: &str,
    ) -> Result<String> {
        Ok("segment".to_string())
    }

    async fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy
    }

    async fn get_primary_algorithm(&self) -> String {
        "segment".to_string()
    }

    fn get_degradation_manager(&self) -> &Arc<DegradationManager> {
        &self.degradation_manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_generator() {
        let generator = MockIdGenerator::new();
        let id = generator.generate("w", "g", "t").await.unwrap();
        // Just verify it doesn't panic
        assert!(id.to_string().parse::<u128>().is_ok());
    }
}
