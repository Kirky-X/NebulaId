// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 真实双 etcd 集群故障转移验证（R-cross-dc-failover-004）。
//!
//! 前置环境：
//! - `ETCD_DC_A_ENDPOINTS`（默认 `http://127.0.0.1:2379`）：primary 集群（dc=1）
//! - `ETCD_DC_B_ENDPOINTS`（默认 `http://127.0.0.1:2380`）：standby 集群（dc=2）
//! - kill 机制 `ETCD_FAILOVER_KILL_CMD`：把 primary 集群打死的整体命令；默认
//!   `docker ps -q --filter name=etcd-a | xargs -r docker kill`（GitHub Actions 的
//!   service 容器与 job 共享 docker daemon，runner 上 docker CLI 可直接定位容器）。
//!
//! 运行：`cargo test --package nebulaid --features etcd,server --test etcd_failover -- --ignored --test-threads=1`
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nebulaid::core::coordinator::{
    DcFailoverController, DcId, DcProbe, EtcdClientOps, EtcdClientWrapper, EtcdClusterStatus,
    FailoverDecision,
};

/// 真实集群探针：对每个 DC 的 etcd 端点做一次 ping，可达即 Healthy / 不可达即 Failed。
struct RealClusterProbe {
    clients: HashMap<DcId, Arc<dyn EtcdClientOps>>,
    probe_timeout: Duration,
}

#[async_trait]
impl DcProbe for RealClusterProbe {
    async fn probe(&self, dc: DcId) -> EtcdClusterStatus {
        match self.clients.get(&dc) {
            Some(client) => {
                // connect 是 lazy 的，可达性必须靠真实 ping 验证；超时按 Failed 判定。
                match tokio::time::timeout(self.probe_timeout, client.ping()).await {
                    Ok(Ok(())) => EtcdClusterStatus::Healthy,
                    _ => EtcdClusterStatus::Failed,
                }
            }
            None => EtcdClusterStatus::Failed,
        }
    }
}

async fn connect_cluster(dc: DcId, endpoints: &str) -> Arc<dyn EtcdClientOps> {
    let wrapper = EtcdClientWrapper::new(vec![endpoints.to_string()])
        .await
        .unwrap_or_else(|e| panic!("dc{dc} 集群连接失败（{endpoints}）: {e}"));
    Arc::new(wrapper)
}

/// kill etcd-a 后 active_dc 必须切至 b：探针回路（refresh → evaluate）驱动真实切换。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "需要双真实 etcd 集群与 kill 机制：CI etcd-integration job（etcd-a/etcd-b 容器）以 --ignored 运行"]
async fn failover_switches_to_healthy_cluster_after_killing_primary() {
    let dc_a_endpoints = std::env::var("ETCD_DC_A_ENDPOINTS")
        .unwrap_or_else(|_| "http://127.0.0.1:2379".to_string());
    let dc_b_endpoints = std::env::var("ETCD_DC_B_ENDPOINTS")
        .unwrap_or_else(|_| "http://127.0.0.1:2380".to_string());

    let mut clients = HashMap::new();
    clients.insert(1u8, connect_cluster(1, &dc_a_endpoints).await);
    clients.insert(2u8, connect_cluster(2, &dc_b_endpoints).await);
    let probe = RealClusterProbe {
        clients,
        probe_timeout: Duration::from_secs(3),
    };

    // 起跑线体检：两集群初始都必须可达（failover 是把流量切到健康集群，
    // 不是掩盖起跑线就不健康的环境）。
    assert_eq!(
        probe.probe(1).await,
        EtcdClusterStatus::Healthy,
        "etcd-a（{dc_a_endpoints}）初始必须可达"
    );
    assert_eq!(
        probe.probe(2).await,
        EtcdClusterStatus::Healthy,
        "etcd-b（{dc_b_endpoints}）初始必须可达"
    );

    let ctl = Arc::new(DcFailoverController::new(1, vec![2]));
    ctl.refresh(&probe).await;
    assert_eq!(ctl.evaluate().await, FailoverDecision::Keep);
    assert_eq!(ctl.active_dc(), 1);

    // kill etcd-a（ETCD_FAILOVER_KILL_CMD 可覆盖；默认经 docker 定位 service 容器）。
    let kill_cmd = std::env::var("ETCD_FAILOVER_KILL_CMD")
        .unwrap_or_else(|_| "docker ps -q --filter name=etcd-a | xargs -r docker kill".to_string());
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&kill_cmd)
        .status()
        .expect("kill 命令必须可执行（需要 docker CLI 与 etcd-a 容器）");
    assert!(status.success(), "kill etcd-a 必须成功：{kill_cmd}");

    // 巡检回路：真实网络下 TCP 半开/连接拒绝需要时间，refresh → evaluate 循环到切换或超时。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut switched = false;
    while tokio::time::Instant::now() < deadline {
        ctl.refresh(&probe).await;
        if matches!(ctl.evaluate().await, FailoverDecision::SwitchTo(2)) {
            switched = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    assert!(
        switched,
        "kill etcd-a 后 60s 内 active_dc 必须切至 b（dc=2）"
    );
    assert_eq!(ctl.active_dc(), 2);
    assert_eq!(ctl.switch_count(), 1, "同一次故障只允许一次切换");

    // 切换目标必须确实健康（防止把流量切进一个同样不可达的集群）。
    ctl.refresh(&probe).await;
    assert_eq!(probe.probe(2).await, EtcdClusterStatus::Healthy);
}
