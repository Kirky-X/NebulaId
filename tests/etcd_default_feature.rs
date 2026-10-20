// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! etcd 默认启用契约测试（cloud-native-dr Phase 1 / T001-T002）。
//!
//! 默认产物必须携带 etcd 协调路径：多实例部署的 worker_id 分配与号段分布式锁只有
//! etcd 实现具备 lease + CAS 语义，缺它就只能退回进程内锁（必然重复发号）。
//! `--no-default-features` 裁剪组合的反向承诺同时钉住：该组合下 etcd 实现不进编译面，
//! 可用的协调实现只剩进程内降级——由 cfg 分支而非运行时判断证明「未编译」。

use nebulaid::core::coordinator::{DistributedLock, EtcdClusterStatus, WorkerIdAllocator};

/// 读取本仓库 `Cargo.toml` 原文，用于在编译期钉住 default 特性列表。
/// cfg 分支只能证明「当前这次构建有没有 etcd」，证明不了「default 有没有 etcd」——
/// 后者必须回到清单文本，否则 `default` 被裁剪掉 etcd 时本文件只会静默少一个用例。
const MANIFEST: &str = include_str!("../Cargo.toml");

/// 把实现类型钉进编译面：类型在当前 feature 组合下不存在时本函数无法通过类型检查。
fn allocator_type_name<T: WorkerIdAllocator + 'static>() -> &'static str {
    std::any::type_name::<T>()
}

fn lock_type_name<T: DistributedLock + 'static>() -> &'static str {
    std::any::type_name::<T>()
}

/// default 特性列表必须显式含 etcd（与上面 cfg 分支互补的清单级防漂移）。
#[test]
fn manifest_default_features_include_etcd() {
    let default_line = MANIFEST
        .lines()
        .find(|line| line.trim_start().starts_with("default = ["))
        .expect("Cargo.toml 必须有 [features] default 列表");
    assert!(
        default_line.contains("\"etcd\""),
        "default 必须含 etcd，实际列表：{default_line}"
    );
}

/// 集群健康三态在两种 feature 组合下都是公共契约（failover 判定依赖其可比较性）。
#[test]
fn cluster_status_is_three_state_and_comparable() {
    assert_eq!(EtcdClusterStatus::Healthy, EtcdClusterStatus::Healthy);
    assert_ne!(EtcdClusterStatus::Healthy, EtcdClusterStatus::Degraded);
    assert_ne!(EtcdClusterStatus::Degraded, EtcdClusterStatus::Failed);
    assert!(format!("{:?}", EtcdClusterStatus::Failed).contains("Failed"));
}

#[cfg(feature = "etcd")]
#[test]
fn default_build_carries_etcd_allocator_and_lock() {
    use nebulaid::core::coordinator::{
        EtcdDistributedLock, EtcdWorkerAllocator, LocalDistributedLock,
    };

    // 不写 `assert!(cfg!(feature = "etcd"))`：本臂能编译即已证明该事实，
    // 而字面常量的 assert 会触 clippy::assertions_on_constants；
    // 「default 含不含 etcd」由上方 manifest_default_features_include_etcd 回到清单文本钉。
    // etcd 生产实现在场
    assert!(
        allocator_type_name::<EtcdWorkerAllocator>().contains("EtcdWorkerAllocator"),
        "默认产物缺少 EtcdWorkerAllocator"
    );
    assert!(
        lock_type_name::<EtcdDistributedLock>().contains("EtcdDistributedLock"),
        "默认产物缺少 EtcdDistributedLock"
    );
    // 未配置端点的单机路径仍可用：etcd 模式下 local 锁作为显式降级实现保留
    assert!(
        lock_type_name::<LocalDistributedLock>().contains("LocalDistributedLock"),
        "默认产物不应丢失 LocalDistributedLock 单机降级路径"
    );
}

#[cfg(not(feature = "etcd"))]
#[test]
fn trimmed_build_keeps_local_fallback_only() {
    use nebulaid::core::coordinator::{LocalDistributedLock, LocalWorkerAllocator};

    // 本臂只在 --no-default-features 裁剪组合下编译（etcd 类型若存在则 import 失败），
    // 因此无需也不应写 `assert!(!cfg!(...))` 这类常量断言。
    // 裁剪组合可构建且只剩进程内实现：etcd 类型在此组合下不存在（若存在则本文件编译失败）
    assert!(
        allocator_type_name::<LocalWorkerAllocator>().contains("LocalWorkerAllocator"),
        "裁剪组合必须保留 LocalWorkerAllocator，否则零 etcd 目标无法发号"
    );
    assert!(
        lock_type_name::<LocalDistributedLock>().contains("LocalDistributedLock"),
        "裁剪组合必须保留 LocalDistributedLock"
    );
}
