// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! NebulaIdCluster Operator 入口：装配 NebulaIdCluster watcher 的 reconcile 回路
//! （owner-watch：CR 派生的 StatefulSet/Service 变更同样触发收敛），退出由
//! `shutdown_on_signal`（SIGTERM/SIGINT）驱动，保证 patch 完整后落停。
//! 探针面（`/healthz` + `/readyz`）由 sdforge health 挂载，见 [`probes`]。

mod controller;
mod crd;
mod probes;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use kube::runtime::watcher::Config as WatcherConfig;
use kube::runtime::Controller;
use kube::{Api, Client};

use crate::controller::{error_policy, reconcile, ControllerContext};
use crate::crd::NebulaIdCluster;
use crate::probes::ReconcileArmed;

/// 探针面绑定地址：默认 `0.0.0.0:8080`（与 operator-deployment.yaml 的
/// containerPort 对齐），`NEBULAID_OPERATOR_PROBE_ADDR` 覆盖；非法取值回退默认并 warn。
fn probe_bind_addr() -> SocketAddr {
    const DEFAULT: SocketAddr = SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        probes::PROBE_PORT,
    );
    match std::env::var("NEBULAID_OPERATOR_PROBE_ADDR") {
        Ok(raw) => match raw.trim().parse() {
            Ok(addr) => addr,
            Err(error) => {
                tracing::warn!(
                    raw = %raw.trim(),
                    %error,
                    "invalid NEBULAID_OPERATOR_PROBE_ADDR; falling back to {DEFAULT}"
                );
                DEFAULT
            }
        },
        Err(_) => DEFAULT,
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // 探针面先于 kube client 起监听：Pod 调度即探活，liveness 不等集群连接建立；
    // 绑定失败（端口被占）显性退出——部署清单的 livenessProbe/readinessProbe
    // 依赖该面，静默缺面会让 Pod 进入无探针的不可观测状态。
    let armed = ReconcileArmed::default();
    let (probe_stop_tx, probe_stop_rx) = tokio::sync::oneshot::channel::<()>();
    let probe_listener = match tokio::net::TcpListener::bind(probe_bind_addr()).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(
                %error,
                "probe server bind failed (override with NEBULAID_OPERATOR_PROBE_ADDR); refusing to run without the probe face"
            );
            return std::process::ExitCode::FAILURE;
        }
    };
    let probe_addr = probe_listener.local_addr().ok();
    let probe_handle =
        match probes::spawn_probe_server(probe_listener, armed.clone(), probe_stop_rx).await {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(%error, "probe router build failed");
                return std::process::ExitCode::FAILURE;
            }
        };
    if let Some(addr) = probe_addr {
        tracing::info!(addr = %addr, "probe face listening on /healthz and /readyz");
    }

    let client = match Client::try_default().await {
        Ok(client) => client,
        Err(e) => {
            tracing::error!(error = %e, "kube client init failed; in-cluster 或 KUBECONFIG 必须可用");
            return std::process::ExitCode::FAILURE;
        }
    };

    let ctx = Arc::new(ControllerContext {
        client: client.clone(),
        http: reqwest::Client::new(),
        probe_timeout: Duration::from_secs(3),
    });

    let clusters: Api<NebulaIdCluster> = Api::all(client.clone());
    tracing::info!("starting nebulaid-operator; watching NebulaIdCluster across namespaces");

    // watcher 即将开始轮询：readiness 置 ready（/readyz 200）。
    armed.arm();

    // owner-watch：CR 之外，其派生的 StatefulSet/Service 变更也入队对应 CR 的 reconcile；
    // shutdown_on_signal 让 SIGTERM/SIGINT 停止入队并等待在途回路收尾。
    Controller::new(clusters, WatcherConfig::default())
        .owns(
            Api::<k8s_openapi::api::apps::v1::StatefulSet>::all(client.clone()),
            WatcherConfig::default(),
        )
        .owns(
            Api::<k8s_openapi::api::core::v1::Service>::all(client.clone()),
            WatcherConfig::default(),
        )
        .shutdown_on_signal()
        .run(reconcile, error_policy, ctx)
        .for_each(|result| async move {
            match result {
                Ok((cluster_ref, _action)) => {
                    tracing::info!(cluster = %cluster_ref.name, "reconciled");
                }
                // controller::Error 的 Display 已含对象引用与错误链（ReconcilerFailed/
                // QueueError/RunnerError），这里统一落 error 日志即可。
                Err(error) => {
                    tracing::error!(error = %error, "reconcile failed");
                }
            }
        })
        .await;

    tracing::info!("nebulaid-operator stopped gracefully");

    // controller 收尾后停探针面：SIGTERM 场景两路停机同源（select 幂等）；
    // controller 先于信号结束的场景由 stop 通道兜底，防 main 返回后探针悬停。
    let _ = probe_stop_tx.send(());
    let _ = probe_handle.await;

    std::process::ExitCode::SUCCESS
}
