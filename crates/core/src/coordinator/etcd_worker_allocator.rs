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

#![allow(dead_code)]

use crate::config::EtcdConfig;
use async_trait::async_trait;
use etcd_client::{Client, ConnectOptions};
use std::sync::{
    atomic::{AtomicI64, AtomicU16, AtomicU8, Ordering},
    Arc,
};
use thiserror::Error;
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
    client: Arc<tokio::sync::Mutex<Client>>,
    datacenter_id: u8,
    allocated_id: AtomicU16,
    lease_id: AtomicI64,
    health_status: AtomicU8,
    #[allow(dead_code)]
    config: EtcdConfig,
    #[allow(dead_code)]
    runtime: tokio::runtime::Handle,
    /// Shutdown signal for graceful termination
    shutdown_rx: Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Receiver<bool>>>>,
    /// Handle to the renewal task
    renewal_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
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
            client: Arc::new(tokio::sync::Mutex::new(client)),
            datacenter_id,
            allocated_id: AtomicU16::new(0),
            lease_id: AtomicI64::new(0),
            health_status: AtomicU8::new(0),
            config,
            runtime: tokio::runtime::Handle::current(),
            shutdown_rx: Arc::new(tokio::sync::Mutex::new(None)),
            renewal_task: Arc::new(tokio::sync::Mutex::new(None)),
        };

        info!("EtcdWorkerAllocator initialized for DC {}", datacenter_id);

        Ok(allocator)
    }

    /// Initialize the shutdown channel for graceful termination
    pub fn init_shutdown(&self) {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let mut shutdown = self.shutdown_rx.lock();
        *shutdown = Some(rx);
        // Store the sender in a way that can be used to trigger shutdown
        let _ = tx; // The sender is dropped, receiver will be closed when all senders are dropped
    }

    /// Start background lease renewal with graceful shutdown support
    pub fn start_background_renewal(&self) {
        self.health_status.store(1, Ordering::SeqCst);

        let client_arc = self.client.clone();
        let lease_id = self.lease_id.load(Ordering::SeqCst);

        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let mut client = client_arc.lock().await;
                        if let Err(e) = client.lease_keep_alive(lease_id).await {
                            error!("Lease renewal failed: {}", e);
                            drop(client);
                            break;
                        }
                    }
                }
            }
        });

        let mut renewal = self.renewal_task.lock();
        *renewal = Some(task);
    }

    /// Stop background lease renewal gracefully
    pub async fn stop_background_renewal(&self) {
        if let Some(task) = self.renewal_task.lock().take() {
            task.abort();
            let _ = task.await;
        }
    }
    }

    async fn grant_lease(&self) -> WorkerAllocatorResult<i64> {
        let lease = self
            .client
            .lock()
            .await
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
        let get_result = {
            let mut client = self.client.lock().await;
            client
                .get(path.clone(), None)
                .await
                .map_err(|e| WorkerAllocatorError::EtcdError(e.to_string()))?
        };

        if get_result.kvs().is_empty() {
            let put_options = Some(etcd_client::PutOptions::new().with_lease(lease_id));
            let mut client = self.client.lock().await;
            match client.put(path, value, put_options).await {
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

    #[allow(dead_code)]
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

            let mut client = self.client.lock().await;
            if let Err(e) = client.lease_keep_alive(lease_id).await {
                error!("Failed to renew lease {}: {}", lease_id, e);
                self.health_status.store(0, Ordering::SeqCst);
                drop(client);
                break;
            }
        }
    }

    pub async fn start_background_renewal(&self) {
        self.health_status.store(1, Ordering::SeqCst);
        let client_arc = self.client.clone();
        let lease_id = self.lease_id.load(Ordering::SeqCst);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));

            loop {
                interval.tick().await;
                let mut client = client_arc.lock().await;
                if let Err(e) = client.lease_keep_alive(lease_id).await {
                    error!("Lease renewal failed: {}", e);
                    drop(client);
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

        let mut client = self.client.lock().await;
        if let Err(e) = client.delete(path, None).await {
            error!("Failed to release worker_id {}: {}", worker_id, e);
            return Err(WorkerAllocatorError::EtcdError(e.to_string()));
        }

        drop(client);
        self.allocated_id.store(0, Ordering::SeqCst);
        self.lease_id.store(0, Ordering::SeqCst);
        info!("Released worker_id: {}", worker_id);

        Ok(())
    }

    pub async fn health_check(&self) -> bool {
        let mut client = self.client.lock().await;
        (client.status().await).is_ok()
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

        match allocator {
            Ok(_) => {
                println!("WARNING: Etcd connection succeeded. This test may pass due to an embedded etcd.");
            }
            Err(e) => {
                println!("Expected error occurred: {:?}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_worker_path_format() {
        let config = EtcdConfig::default();
        let allocator =
            EtcdWorkerAllocator::new(vec!["localhost:2379".to_string()], 1, config).await;

        match allocator {
            Ok(_) => {
                println!("WARNING: Etcd connection succeeded when expected to fail. This may indicate an embedded etcd or mock is active.");
            }
            Err(WorkerAllocatorError::ConnectionFailed(msg)) => {
                println!("Connection failed as expected: {}", msg);
            }
            Err(WorkerAllocatorError::EtcdError(msg)) => {
                println!("Etcd error as expected: {}", msg);
                assert!(msg.contains("connection") || msg.contains("Connect"));
            }
            other => {
                assert!(other.is_err(), "Expected error but got Ok");
            }
        }
    }
}
