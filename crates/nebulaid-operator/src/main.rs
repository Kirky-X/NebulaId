// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! NebulaIdCluster Operator 入口：装配 NebulaIdCluster watcher 的 reconcile 回路
//! （owner-watch：CR 派生的 StatefulSet/Service 变更同样触发收敛），退出由
//! `shutdown_on_signal`（SIGTERM/SIGINT）驱动，保证 patch 完整后落停。

mod controller;
mod crd;

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use kube::runtime::watcher::Config as WatcherConfig;
use kube::runtime::Controller;
use kube::{Api, Client};

use crate::controller::{error_policy, reconcile, ControllerContext};
use crate::crd::NebulaIdCluster;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

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
    std::process::ExitCode::SUCCESS
}
