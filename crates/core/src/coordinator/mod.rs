pub mod etcd_worker_allocator;

pub use etcd_worker_allocator::{
    EtcdWorkerAllocator, WorkerAllocatorError, WorkerAllocatorResult, WorkerIdAllocator,
};
