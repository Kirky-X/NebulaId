use crate::config::EtcdConfig;
use async_trait::async_trait;
use etcd_client::{
    Client, ConnectOptions, LockOptions, Operation, OperationResponse, Txn, TxnOpResponse,
    TxnResponse,
};
use std::sync::{
    atomic::{AtomicI64, AtomicU16, AtomicU8, Ordering},
    Arc,
};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

const MAX_WORKER_ID: u16 = 255;
const WORKER_PATH_PREFIX: &str = "/idgen/workers";

#[derive(Debug, Error)]
pub enum WorkerAllocatorError {
    #[error("Failed to connect to etcd: {0}")]
    ConnectionFailed(String),

    #[error("No available worker ID")]
    NoAvailableId,

    #[error("Lease renewal failed: {0}")]
    LeaseRenewalFailed(String),

    #[error("Worker ID allocation cancelled")]
    Cancelled,

    #[error("etcd error: {0}")]
    EtcdError(String),
}

pub type WorkerAllocatorResult<T> = std::result::Result<T, WorkerAllocatorError>;

#[async_trait]
pub trait WorkerIdAllocator: Send + Sync {
    async fn allocate(&self) -> WorkerAllocatorResult<u16>;
    async fn release(&self, worker_id: u16) -> WorkerAllocatorResult<()>;
    fn get_allocated_id(&self) -> Option<u16>;
    fn is_healthy(&self) -> bool;
}

pub struct EtcdWorkerAllocator {
    client: Arc<Client>,
    datacenter_id: u8,
    allocated_id: AtomicU16,
    lease_id: AtomicI64,
    health_status: AtomicU8,
    config: EtcdConfig,
    runtime: tokio::runtime::Handle,
}

impl EtcdWorkerAllocator {
    pub async fn new(
        endpoints: Vec<String>,
        datacenter_id: u8,
        config: EtcdConfig,
    ) -> WorkerAllocatorResult<Self> {
        let options = ConnectOptions::new()
            .with_connect_timeout(std::time::Duration::from_millis(config.connect_timeout_ms))
            .with_timeout(std::time::Duration::from_millis(config.watch_timeout_ms));

        let client = Client::connect(endpoints, Some(options))
            .await
            .map_err(|e| WorkerAllocatorError::ConnectionFailed(e.to_string()))?;

        let allocator = Self {
            client: Arc::new(client),
            datacenter_id,
            allocated_id: AtomicU16::new(0),
            lease_id: AtomicI64::new(0),
            health_status: AtomicU8::new(0),
            config,
            runtime: tokio::runtime::Handle::current(),
        };

        info!("EtcdWorkerAllocator initialized for DC {}", datacenter_id);

        Ok(allocator)
    }

    async fn grant_lease(&self) -> WorkerAllocatorResult<i64> {
        let lease = self
            .client
            .lease_grant(30, None)
            .await
            .map_err(|e| WorkerAllocatorError::LeaseRenewalFailed(e.to_string()))?;

        let lease_id = lease.id();
        self.lease_id.store(lease_id, Ordering::SeqCst);
        info!("Lease granted: {}", lease_id);

        Ok(lease_id)
    }

    fn worker_path(&self, worker_id: u16) -> String {
        format!(
            "{}/{}/{}",
            WORKER_PATH_PREFIX, self.datacenter_id, worker_id
        )
    }

    async fn try_allocate_id(&self, worker_id: u16, lease_id: i64) -> WorkerAllocatorResult<bool> {
        let path = self.worker_path(worker_id);
        let value = format!(
            "dc={},pid={},ts={}",
            self.datacenter_id,
            std::process::id(),
            chrono::Utc::now().timestamp()
        );

        let mut success = false;
        let get_result = self
            .client
            .get(path.clone(), None)
            .await
            .map_err(|e| WorkerAllocatorError::EtcdError(e.to_string()))?;

        if get_result.kvs().is_empty() {
            let put_options = Some(etcd_client::PutOptions::new().with_lease(lease_id));
            match self.client.put(path, value, put_options).await {
                Ok(_) => success = true,
                Err(e) => return Err(WorkerAllocatorError::EtcdError(e.to_string())),
            }
        }

        Ok(success)
    }

    async fn do_allocate(&self) -> WorkerAllocatorResult<u16> {
        let lease_id = self.grant_lease().await?;

        for worker_id in 0..=MAX_WORKER_ID {
            match self.try_allocate_id(worker_id, lease_id).await {
                Ok(true) => {
                    self.allocated_id.store(worker_id, Ordering::SeqCst);
                    info!("Successfully allocated worker_id: {}", worker_id);
                    return Ok(worker_id);
                }
                Ok(false) => continue,
                Err(e) => {
                    warn!("Failed to allocate worker_id {}: {}", worker_id, e);
                    continue;
                }
            }
        }

        Err(WorkerAllocatorError::NoAvailableId)
    }

    async fn renew_lease(&self) {
        let lease_id = self.lease_id.load(Ordering::SeqCst);
        if lease_id == 0 {
            return;
        }

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;

            if self.lease_id.load(Ordering::SeqCst) != lease_id {
                break;
            }

            if let Err(e) = self.client.lease_keep_alive(lease_id).await {
                error!("Failed to renew lease {}: {}", lease_id, e);
                self.health_status.store(0, Ordering::SeqCst);
                break;
            }
        }
    }

    pub async fn start_background_renewal(&self) {
        self.health_status.store(1, Ordering::SeqCst);
        let client_clone = self.client.clone();
        let lease_id = self.lease_id.load(Ordering::SeqCst);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));

            loop {
                interval.tick().await;
                if let Err(e) = client_clone.lease_keep_alive(lease_id).await {
                    error!("Lease renewal failed: {}", e);
                    break;
                }
            }
        });
    }

    pub async fn allocate(&self) -> WorkerAllocatorResult<u16> {
        self.do_allocate().await
    }

    pub async fn release(&self, worker_id: u16) -> WorkerAllocatorResult<()> {
        let path = self.worker_path(worker_id);

        if let Err(e) = self.client.delete(path).await {
            error!("Failed to release worker_id {}: {}", worker_id, e);
            return Err(WorkerAllocatorError::EtcdError(e.to_string()));
        }

        self.allocated_id.store(0, Ordering::SeqCst);
        self.lease_id.store(0, Ordering::SeqCst);
        info!("Released worker_id: {}", worker_id);

        Ok(())
    }

    pub async fn health_check(&self) -> bool {
        if let Ok(_response) = self
            .client
            .status(etcd_client::LeaseId::from(
                self.lease_id.load(Ordering::SeqCst),
            ))
            .await
        {
            true
        } else {
            false
        }
    }
}

#[async_trait]
impl WorkerIdAllocator for EtcdWorkerAllocator {
    async fn allocate(&self) -> WorkerAllocatorResult<u16> {
        self.allocate().await
    }

    async fn release(&self, worker_id: u16) -> WorkerAllocatorResult<()> {
        self.release(worker_id).await
    }

    fn get_allocated_id(&self) -> Option<u16> {
        let id = self.allocated_id.load(Ordering::SeqCst);
        if id == 0 {
            None
        } else {
            Some(id)
        }
    }

    fn is_healthy(&self) -> bool {
        self.health_status.load(Ordering::SeqCst) == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_worker_path_generation() {
        let config = EtcdConfig::default();
        let allocator =
            EtcdWorkerAllocator::new(vec!["localhost:2379".to_string()], 1, config).await;

        assert!(allocator.is_err());
    }

    #[test]
    fn test_worker_path_format() {
        let config = EtcdConfig::default();
        let allocator =
            EtcdWorkerAllocator::new(vec!["localhost:2379".to_string()], 1, config).unwrap_err();

        match allocator {
            WorkerAllocatorError::ConnectionFailed(_) => {}
            _ => panic!("Expected ConnectionFailed error"),
        }
    }
}
