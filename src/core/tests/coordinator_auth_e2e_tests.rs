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

#![cfg(test)]

//! # 协调与认证模块端到端测试（coordinator + auth e2e）
//!
//! 覆盖《功能场景穷举分析》第 2.4 节（分布式协调）和第 2.1 节
//! （core/auth 模块：from_env / hash_key / validate_key）的端到端场景。
//!
//! ## 测试分组
//!
//! - **E2E-AUTHCORE 组**（core/auth 模块 e2e）：AuthManager::from_env
//!   在 dev 模式自动生成 salt；hash_key 产生可被 argon2 验证的 PHC 格式
//!   hash；不同输入产生不同 hash；validate_key 正确/错误/禁用密钥行为；
//!   缓存命中 vs 缺失的行为差异。
//! - **E2E-LOCAL 组**（本地缓存 e2e）：EtcdClusterHealthMonitor 的
//!   save/load_local_cache round-trip、文件不存在返回 Ok、损坏文件返回 Err。
//!   注：这些方法是 EtcdClusterHealthMonitor 的方法，仅在 `etcd` feature 下
//!   有真实文件 IO 实现（no-etcd 是 stub 永远返回 Ok），因此用
//!   `#[cfg(feature = "etcd")]` 门控。
//! - **E2E-ETCD-MOCK 组**（etcd 协调 mock e2e）：EtcdWorkerAllocator 分配
//!   唯一 worker_id；EtcdDistributedLock acquire + release；
//!   EtcdClusterHealthMonitor 记录失败 → 3 次 Degraded / 5 次 Failed。
//!   用 `#[cfg(feature = "etcd")]` 门控，通过 mockall 注入 mock 客户端
//!   避免依赖真实 etcd 集群。
//!
//! ## 并行安全
//!
//! - 涉及环境变量的测试用 `ENV_MUTEX` 串行化，避免并行测试间 env var 互相污染。
//! - 文件 IO 测试用 `tempfile::NamedTempFile` 隔离，每个测试独立临时文件。
//! - mock 客户端每次测试独立构造，无共享状态。

use std::sync::Mutex;
use std::time::Duration;

use crate::core::auth::{AuthConfig, AuthManager};

// =============================================================================
// 串行化 env var 访问的 mutex
// =============================================================================

/// 串行化所有读取/写入环境变量的测试，避免并行污染。
/// 测试函数开头 `let _guard = ENV_MUTEX.lock().unwrap();` 持有至函数结束。
static ENV_MUTEX: Mutex<()> = Mutex::new(());

// =============================================================================
// E2E-AUTHCORE 组：core/auth 模块端到端
// =============================================================================
//
// AuthManager::from_env 在生产模式（NEBULA_ENV=production）下缺
// NEBULA_API_KEY_SALT 会 panic；dev 模式下会自动生成 32 字节随机 salt
// （getrandom 失败时回退到固定 fallback_dev_salt_not_for_production）。
// hash_key 是私有方法，无法直接调用，通过 add_key + validate_key 间接验证：
// - add_key 内部调用 hash_key 产生 PHC 格式 hash 存入 ApiKeyData.key_hash
// - validate_key 内部调用 verify_key (argon2::verify_password) 验证 PHC hash
// 若 hash_key 不产生 PHC 格式，verify_password 会失败，validate_key 返回 None。

/// dev 模式（NEBULA_ENV 未设或非 production）下 from_env 应自动生成非空 salt。
#[tokio::test]
async fn e2e_auth_manager_from_env_dev_mode_generates_salt() {
    let _guard = ENV_MUTEX.lock().unwrap();
    // 清除环境变量，确保进入 dev 模式且不使用预设 salt
    std::env::remove_var("NEBULA_ENV");
    std::env::remove_var("NEBULA_API_KEY_SALT");

    let config = AuthConfig::from_env();
    assert!(
        !config.salt.is_empty(),
        "dev 模式下 from_env 应生成非空 salt，实际: {:?}",
        config.salt
    );
    // dev 模式生成的 salt 应为 32 字节 hex（64 字符）或固定 fallback
    assert!(
        config.salt.len() == 64 || config.salt == "fallback_dev_salt_not_for_production",
        "dev 模式 salt 应为 64 字符 hex 或 fallback，实际长度: {}",
        config.salt.len()
    );
}

/// hash_key 应产生 PHC 格式 hash（间接验证：add_key 后 validate_key 成功）。
///
/// hash_key 是私有方法，无法直接调用。通过 add_key 间接调用 hash_key 产生
/// PHC hash 存入 cache，再用 validate_key 调用 argon2::verify_password 验证。
/// argon2::verify_password 只能验证 PHC 格式字符串，validate_key 返回 Some
/// 即证明 hash_key 产生了有效 PHC 格式 hash。
#[tokio::test]
async fn e2e_auth_manager_hash_key_returns_phc_format() {
    let config = AuthConfig {
        salt: "test_salt_for_phc".to_string(),
        cache_ttl_seconds: 300,
        max_cache_size: std::num::NonZeroUsize::new(100).unwrap(),
    };
    let manager = AuthManager::new(config).await;

    let key_id = manager
        .add_key(
            "phc-test-key".to_string(),
            "phc-test-secret".to_string(),
            "workspace-phc".to_string(),
            None,
            vec![],
        )
        .await
        .expect("add_key 应成功");

    // validate_key 内部走 verify_key → argon2::verify_password(password, &parsed)
    // 其中 parsed = PasswordHash::new(stored_hash)。若 stored_hash 非 PHC 格式，
    // PasswordHash::new 返回 Err → verify_key 返回 false → validate_key 返回 None。
    let workspace = manager.validate_key(&key_id, "phc-test-secret").await;
    assert_eq!(
        workspace,
        Some("workspace-phc".to_string()),
        "validate_key 成功即证明 hash_key 产生了有效 PHC 格式 hash"
    );
}

/// 不同输入（key_id + key_secret）应产生不同 hash，密钥不可互换验证。
#[tokio::test]
async fn e2e_auth_manager_hash_key_different_inputs_different_hashes() {
    let config = AuthConfig {
        salt: "shared_salt".to_string(),
        cache_ttl_seconds: 300,
        max_cache_size: std::num::NonZeroUsize::new(100).unwrap(),
    };
    let manager = AuthManager::new(config).await;

    let key_id1 = manager
        .add_key(
            "key-aaa".to_string(),
            "secret-111".to_string(),
            "workspace-A".to_string(),
            None,
            vec![],
        )
        .await
        .unwrap();
    let key_id2 = manager
        .add_key(
            "key-bbb".to_string(),
            "secret-222".to_string(),
            "workspace-B".to_string(),
            None,
            vec![],
        )
        .await
        .unwrap();

    // 交叉验证：key1 只能用 secret1，key2 只能用 secret2
    // 若 hash_key 对不同输入产生相同 hash，则交叉验证会成功
    assert_eq!(
        manager.validate_key(&key_id1, "secret-111").await,
        Some("workspace-A".to_string()),
        "key1 + 正确 secret 应验证成功"
    );
    assert_eq!(
        manager.validate_key(&key_id1, "secret-222").await,
        None,
        "key1 + 错误 secret 应验证失败（hash 不同）"
    );
    assert_eq!(
        manager.validate_key(&key_id2, "secret-222").await,
        Some("workspace-B".to_string()),
        "key2 + 正确 secret 应验证成功"
    );
    assert_eq!(
        manager.validate_key(&key_id2, "secret-111").await,
        None,
        "key2 + 错误 secret 应验证失败（hash 不同）"
    );
}

/// validate_key 用正确密钥应返回 workspace_id。
#[tokio::test]
async fn e2e_auth_manager_validate_key_correct_secret_returns_workspace_id() {
    let config = AuthConfig {
        salt: "salt_correct".to_string(),
        cache_ttl_seconds: 300,
        max_cache_size: std::num::NonZeroUsize::new(100).unwrap(),
    };
    let manager = AuthManager::new(config).await;

    let key_id = manager
        .add_key(
            "correct-key".to_string(),
            "correct-secret".to_string(),
            "workspace-xyz-789".to_string(),
            None,
            vec!["read".to_string(), "write".to_string()],
        )
        .await
        .unwrap();

    let result = manager.validate_key(&key_id, "correct-secret").await;
    assert_eq!(
        result,
        Some("workspace-xyz-789".to_string()),
        "正确密钥应返回对应 workspace_id"
    );
}

/// validate_key 用错误密钥应返回 None。
#[tokio::test]
async fn e2e_auth_manager_validate_key_wrong_secret_returns_none() {
    let config = AuthConfig {
        salt: "salt_wrong".to_string(),
        cache_ttl_seconds: 300,
        max_cache_size: std::num::NonZeroUsize::new(100).unwrap(),
    };
    let manager = AuthManager::new(config).await;

    let key_id = manager
        .add_key(
            "wrong-test-key".to_string(),
            "actual-secret".to_string(),
            "workspace-should-not-return".to_string(),
            None,
            vec![],
        )
        .await
        .unwrap();

    let result = manager.validate_key(&key_id, "wrong-secret").await;
    assert_eq!(result, None, "错误密钥应返回 None，不应泄露 workspace_id");
}

/// validate_key 对已禁用（revoke）的密钥应返回 None。
#[tokio::test]
async fn e2e_auth_manager_validate_key_disabled_key_returns_none() {
    let config = AuthConfig {
        salt: "salt_disabled".to_string(),
        cache_ttl_seconds: 300,
        max_cache_size: std::num::NonZeroUsize::new(100).unwrap(),
    };
    let manager = AuthManager::new(config).await;

    let key_id = manager
        .add_key(
            "disable-test-key".to_string(),
            "disable-secret".to_string(),
            "workspace-disabled".to_string(),
            None,
            vec![],
        )
        .await
        .unwrap();

    // 吊用前应能验证
    assert_eq!(
        manager.validate_key(&key_id, "disable-secret").await,
        Some("workspace-disabled".to_string()),
        "吊用前应能验证"
    );

    // revoke_key 将 enabled 设为 false
    let revoked = manager.revoke_key(&key_id).await;
    assert!(revoked, "revoke_key 应返回 true（key 存在）");

    // 吊用后 validate_key 应返回 None（enabled=false 早返回）
    let result = manager.validate_key(&key_id, "disable-secret").await;
    assert_eq!(result, None, "禁用密钥应返回 None，即使密钥本身正确");
}

/// 缓存命中应使 validate_key 返回 workspace_id；缓存缺失（clear_cache）后返回 None。
///
/// AuthManager 没有持久化存储，ApiKeyData 仅存在 oxcache 中。
/// - add_key 后 cache 中有 key → validate_key 命中缓存返回 Some
/// - clear_cache 后 cache 中无 key → validate_key 未命中返回 None
/// 这验证了 cache 在 validate_key 路径中的关键作用。
#[tokio::test]
async fn e2e_auth_manager_cache_hit_speeds_up_validation() {
    let config = AuthConfig {
        salt: "salt_cache_test".to_string(),
        cache_ttl_seconds: 300,
        max_cache_size: std::num::NonZeroUsize::new(100).unwrap(),
    };
    let manager = AuthManager::new(config).await;

    let key_id = manager
        .add_key(
            "cache-test-key".to_string(),
            "cache-secret".to_string(),
            "workspace-cache".to_string(),
            None,
            vec![],
        )
        .await
        .unwrap();

    // 缓存命中：add_key 已写入 cache，validate_key 应返回 Some
    let cache_hit = manager.validate_key(&key_id, "cache-secret").await;
    assert_eq!(
        cache_hit,
        Some("workspace-cache".to_string()),
        "缓存命中时 validate_key 应返回 workspace_id"
    );

    // 清除缓存：模拟 cache miss（无持久化存储）
    manager.clear_cache().await;
    // 等待 oxcache 异步清理完成
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 缓存未命中：validate_key 在 cache 中找不到 key → 返回 None
    let cache_miss = manager.validate_key(&key_id, "cache-secret").await;
    assert_eq!(
        cache_miss, None,
        "缓存未命中且无持久化存储时 validate_key 应返回 None，证明 cache 在路径中起作用"
    );
}

// =============================================================================
// E2E-LOCAL 组：本地缓存端到端（仅 etcd feature 下有真实文件 IO）
// =============================================================================
//
// 注：load_local_cache / save_local_cache 是 EtcdClusterHealthMonitor 的方法。
// - `etcd` feature：真实文件 IO（fs::read_to_string / fs::write + serde_json）
// - `not(etcd)` feature：stub 永远返回 Ok(())，不触碰文件系统
// 因此本组测试用 `#[cfg(feature = "etcd")]` 门控，仅在 etcd feature 下运行。

#[cfg(feature = "etcd")]
mod etcd_local_cache_tests {
    use crate::core::config::EtcdConfig;
    use crate::core::coordinator::EtcdClusterHealthMonitor;
    use tempfile::NamedTempFile;

    /// save + load round-trip：保存后加载应得到一致的缓存数据。
    #[tokio::test]
    async fn e2e_local_cache_save_and_load_roundtrip() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();

        // 第一个 monitor：写入数据并 save
        let monitor = EtcdClusterHealthMonitor::new(config.clone(), cache_path.clone());
        monitor.put_to_cache("rt-key-1".to_string(), "rt-value-1".to_string(), 1);
        monitor.put_to_cache("rt-key-2".to_string(), "rt-value-2".to_string(), 2);
        monitor
            .save_local_cache()
            .await
            .expect("save_local_cache 应成功");

        // 第二个 monitor：从同一文件 load
        let monitor2 = EtcdClusterHealthMonitor::new(config, cache_path);
        monitor2
            .load_local_cache()
            .await
            .expect("load_local_cache 应成功");

        // 验证 round-trip 数据一致
        let entry1 = monitor2.get_from_cache("rt-key-1");
        let entry2 = monitor2.get_from_cache("rt-key-2");
        assert!(entry1.is_some(), "rt-key-1 应存在");
        assert!(entry2.is_some(), "rt-key-2 应存在");
        assert_eq!(entry1.unwrap().value, "rt-value-1");
        assert_eq!(entry2.unwrap().value, "rt-value-2");
    }

    /// load 不存在的文件应返回 Ok(())（早返回，不报错）。
    #[tokio::test]
    async fn e2e_local_cache_load_nonexistent_returns_ok() {
        let config = EtcdConfig::default();
        // 使用不存在的路径
        let cache_path = "/nonexistent/path/that/does/not/exist.json".to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        let result = monitor.load_local_cache().await;
        assert!(
            result.is_ok(),
            "文件不存在时应返回 Ok(()) 而非 Err, 实际: {:?}",
            result.err()
        );
    }

    /// load 损坏（非 JSON）的文件应返回 Err。
    #[tokio::test]
    async fn e2e_local_cache_load_corrupted_returns_error() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        // 写入无效 JSON
        std::fs::write(cache_file.path(), "this is not valid json [[[").unwrap();

        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);
        let result = monitor.load_local_cache().await;
        assert!(result.is_err(), "损坏文件应返回 Err, 实际: {:?}", result);
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("parse") || err_msg.contains("Failed to parse"),
            "错误消息应提及 parse, 实际: {}",
            err_msg
        );
    }
}

// =============================================================================
// E2E-ETCD-MOCK 组：etcd 协调 mock 端到端（仅 etcd feature 下编译）
// =============================================================================
//
// 通过 mockall 注入 MockEtcdClientOps，避免依赖真实 etcd 集群。
// etcd.rs 内部已有 mockall::mock! 定义，但定义在 `#[cfg(test)] mod tests`
// 内部无法跨模块引用，因此这里重新定义一份独立 mock。

#[cfg(feature = "etcd")]
mod etcd_coord_mock_tests {
    use crate::core::config::EtcdConfig;
    use crate::core::coordinator::{
        DistributedLock, EtcdClientOps, EtcdClusterHealthMonitor, EtcdClusterStatus,
        EtcdDistributedLock, EtcdError, EtcdWorkerAllocator, WorkerIdAllocator,
    };
    use async_trait::async_trait;
    use std::sync::Arc;
    use tempfile::NamedTempFile;

    mockall::mock! {
        pub EtcdClientOps {}

        #[async_trait]
        impl EtcdClientOps for EtcdClientOps {
            async fn kv_get(&self, key: &str) -> std::result::Result<Option<Vec<u8>>, EtcdError>;
            async fn kv_delete(&self, key: &str) -> std::result::Result<(), EtcdError>;
            async fn lease_grant(&self, ttl: i64) -> std::result::Result<i64, EtcdError>;
            async fn lease_revoke(&self, lease_id: i64) -> std::result::Result<(), EtcdError>;
            async fn txn_check_create_rev_and_put(
                &self,
                key: &str,
                value: Vec<u8>,
                lease_id: i64,
            ) -> std::result::Result<bool, EtcdError>;
            async fn ping(&self) -> std::result::Result<(), EtcdError>;
        }
    }

    /// 把 mock 封装为 `Arc<dyn EtcdClientOps>`，方便各测试复用。
    fn mock_into_client(mock: MockEtcdClientOps) -> Arc<dyn EtcdClientOps> {
        Arc::new(mock)
    }

    /// EtcdWorkerAllocator::allocate 应分配到唯一的 worker_id（>= 1）。
    ///
    /// mock 行为：所有 key 不存在（kv_get → None），CAS 成功（txn → true），
    /// lease_grant 成功。期望分配 worker_id=1（从 1 开始跳过 0）。
    #[tokio::test]
    async fn e2e_etcd_worker_allocator_allocate_returns_unique_id() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|_| Ok(None));
        mock.expect_lease_grant().returning(|_| Ok(123));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .expect("allocator new 应成功");

        let worker_id = allocator.allocate().await.expect("allocate 应成功");
        assert!(
            worker_id >= 1,
            "分配的 worker_id 应 >= 1 (跳过 0 作为未分配哨兵), 实际: {}",
            worker_id
        );
        assert_eq!(
            allocator.get_allocated_id(),
            Some(worker_id),
            "get_allocated_id 应返回刚分配的 id"
        );
    }

    /// EtcdDistributedLock::acquire + guard.release 完整流程应成功。
    ///
    /// mock 行为：lease_grant 成功，CAS 成功（首次获取），release 时
    /// lease_revoke 成功。验证 acquire → release 端到端协同。
    #[tokio::test]
    async fn e2e_etcd_distributed_lock_acquire_and_release() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant().returning(|_| Ok(456));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        // 显式 release 成功 → released=true → drop 跳过，lease_revoke 仅调用 1 次
        mock.expect_lease_revoke().times(1).returning(|_| Ok(()));

        let lock = EtcdDistributedLock::new(mock_into_client(mock), "/e2e-locks/".to_string())
            .await
            .expect("lock new 应成功");

        // acquire
        let guard = lock.acquire("e2e-lock-key", 5).await;
        assert!(guard.is_ok(), "acquire 应成功, 实际: {:?}", guard.err());

        // 显式 release
        let guard = guard.unwrap();
        let release_result = guard.release().await;
        assert!(
            release_result.is_ok(),
            "显式 release 应成功, 实际: {:?}",
            release_result.err()
        );
        // guard drop 时 released=true → Drop 跳过 lease_revoke
    }

    /// EtcdClusterHealthMonitor::record_failure 应累计失败计数。
    ///
    /// 1-2 次 record_failure 不应改变 Healthy 状态（阈值 >= 3 才 Degraded）。
    /// 验证 failure_count 内部累计（通过状态转换间接验证）。
    #[tokio::test]
    async fn e2e_etcd_cluster_health_monitor_records_failures() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        // 初始 Healthy
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
        assert!(!monitor.is_using_cache());

        // 1 次失败：仍为 Healthy（< 3 阈值）
        monitor.record_failure();
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Healthy,
            "1 次失败不应降级"
        );

        // 2 次失败：仍为 Healthy
        monitor.record_failure();
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Healthy,
            "2 次失败不应降级"
        );
    }

    /// EtcdClusterHealthMonitor 连续 3 次 record_failure 应进入 Degraded 状态。
    ///
    /// 覆盖 etcd.rs record_failure 第 279-287 行：consecutive_failures >= 3
    /// 且 < 5 时 set_status(Degraded)。Degraded 状态不应启用本地缓存降级
    /// （仅 Failed 状态才 is_using_cache=true）。
    #[tokio::test]
    async fn e2e_etcd_cluster_health_monitor_degraded_after_3_failures() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        // 3 次失败 → Degraded
        monitor.record_failure();
        monitor.record_failure();
        monitor.record_failure();
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Degraded,
            "连续 3 次失败应进入 Degraded"
        );
        assert!(
            !monitor.is_using_cache(),
            "Degraded 状态不应启用本地缓存（仅 Failed 才启用）"
        );

        // 继续到 5 次失败 → Failed
        monitor.record_failure();
        monitor.record_failure();
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Failed,
            "连续 5 次失败应进入 Failed"
        );
        assert!(monitor.is_using_cache(), "Failed 状态应启用本地缓存降级");
    }
}
