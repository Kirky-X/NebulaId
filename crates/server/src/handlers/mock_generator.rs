use async_trait::async_trait;
use nebula_core::algorithm::{IdAlgorithm, IdGenerator as CoreIdGenerator};
use nebula_core::{Result, Id};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct MockIdGenerator {
    counter: Arc<std::sync::atomic::AtomicU64>,
}

impl MockIdGenerator {
    pub fn new() -> Self {
        Self {
            counter: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }
}

#[async_trait]
impl CoreIdGenerator for MockIdGenerator {
    async fn generate(&self, _workspace: &str, _group: &str, _biz_tag: &str) -> Result<Id> {
        let id = self.counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Id::Numeric(id))
    }

    async fn batch_generate(&self, _workspace: &str, _group: &str, _biz_tag: &str, size: usize) -> Result<Vec<Id>> {
        let start = self.counter.fetch_add(size as u64, std::sync::atomic::Ordering::SeqCst);
        Ok((start + 1..=start + size as u64).map(Id::Numeric).collect())
    }

    async fn get_algorithm_name(&self, _workspace: &str, _group: &str, _biz_tag: &str) -> Result<String> {
        Ok("segment".to_string())
    }

    async fn health_check(&self) -> nebula_core::HealthStatus {
        nebula_core::HealthStatus::healthy()
    }

    async fn get_primary_algorithm(&self) -> String {
        "segment".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_generator() {
        let generator = MockIdGenerator::new();
        let id = generator.generate("w", "g", "t").await.unwrap();
        assert!(matches!(id, Id::Numeric(_)));
    }
}
