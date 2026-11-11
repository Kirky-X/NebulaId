// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 探针面：sdforge health 挂载 `/healthz`（liveness）与 `/readyz`（readiness）。
//!
//! readiness 的唯一检查项是 reconcile 回路是否已武装（watcher 即将开始轮询）：
//! kube client 初始化失败在 main 直接退出，不存在「进程存活但回路未建」的中间态，
//! 因此武装前 `/readyz` 503、武装后 200。探针路由由 sdforge 在认证层之后挂载，
//! 构造即免认证。reconcile 主循环不经本模块。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 探针面默认端口（与 `deploy/manifest/operator-deployment.yaml` 的
/// `containerPort: 8080` 对齐；绑定地址经 `NEBULAID_OPERATOR_PROBE_ADDR` 覆盖）。
pub const PROBE_PORT: u16 = 8080;

/// reconcile 回路已武装信号：Clone 共享同一原子，主流程 [`ReconcileArmed::arm`]
/// 置位后 `/readyz` 的 `reconcile-loop` 检查转健康。
#[derive(Clone, Default)]
pub struct ReconcileArmed(Arc<AtomicBool>);

impl ReconcileArmed {
    /// 武装：watcher 即将开始轮询时由 main 调用一次。
    pub fn arm(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn armed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// 在已绑定的 listener 上启动探针面服务，返回任务句柄。
///
/// 停机触发双路 `select`：SIGTERM/SIGINT（`default_shutdown_signal`，与
/// kube-runtime `shutdown_on_signal` 同源信号，tokio 允许多监听器互不抢占）
/// 或调用方发出 `stop`（controller 先于信号结束时的兜底，防进程滞留）。
pub async fn spawn_probe_server(
    listener: tokio::net::TcpListener,
    armed: ReconcileArmed,
    stop: tokio::sync::oneshot::Receiver<()>,
) -> std::io::Result<tokio::task::JoinHandle<()>> {
    // readiness 检查（进程级注册表，单测先 clear_readiness_checks 隔离）。
    sdforge::health::register_readiness_check_fn("reconcile-loop", move || {
        if armed.armed() {
            sdforge::health::CheckOutcome::healthy("reconcile-loop")
        } else {
            sdforge::health::CheckOutcome::unhealthy(
                "reconcile-loop",
                "reconcile loop not armed yet",
            )
        }
    });
    let config = sdforge::config::SdForgeConfig::default();
    let router = sdforge::http::build_with_config_ext(sdforge::axum::Router::new(), &config)
        .map_err(std::io::Error::other)?;
    Ok(tokio::spawn(async move {
        if let Err(error) = sdforge::http::serve_with_graceful_shutdown(
            router,
            listener,
            async {
                tokio::select! {
                    _ = sdforge::http::default_shutdown_signal() => {}
                    _ = stop => {}
                }
            },
            sdforge::http::GracefulShutdownConfig::default(),
        )
        .await
        {
            tracing::error!(%error, "probe server terminated with error");
        }
        tracing::info!("probe server stopped");
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 探针面全链路（真实 loopback HTTP）：/healthz 恒 200；/readyz 武装前 503、
    /// arm 后 200；stop 通道发出后服务收尾退出。
    #[tokio::test]
    async fn probe_face_serves_healthz_and_readyz_lifecycle() {
        // sdforge readiness 注册表是进程级全局：清空后注册本用例检查项。
        sdforge::health::clear_readiness_checks();

        let armed = ReconcileArmed::default();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let listener =
            tokio::net::TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
                .await
                .expect("ephemeral bind must succeed");
        let addr = listener.local_addr().expect("local_addr must be available");
        let handle = spawn_probe_server(listener, armed.clone(), stop_rx)
            .await
            .expect("probe server must start");

        let http = reqwest::Client::new();

        // liveness：恒 200 + status healthy（未武装也不受影响——进程活着即可）。
        let resp = http
            .get(format!("http://{addr}/healthz"))
            .send()
            .await
            .expect("GET /healthz must reach the probe face");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&resp.text().await.expect("healthz body must be readable"))
                .expect("healthz body must be JSON");
        assert_eq!(body["status"], "healthy");

        // readiness：武装前 503，检查项为 reconcile-loop。
        let resp = http
            .get(format!("http://{addr}/readyz"))
            .send()
            .await
            .expect("GET /readyz must reach the probe face");
        assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value =
            serde_json::from_str(&resp.text().await.expect("readyz body must be readable"))
                .expect("readyz body must be JSON");
        assert_eq!(body["status"], "unavailable");
        assert_eq!(body["checks"][0]["name"], "reconcile-loop");

        // arm 后转 ready。
        armed.arm();
        let resp = http
            .get(format!("http://{addr}/readyz"))
            .send()
            .await
            .expect("GET /readyz (armed) must reach the probe face");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&resp.text().await.expect("readyz body must be readable"))
                .expect("readyz body must be JSON");
        assert_eq!(body["status"], "ready");

        // 停机兜底通道：controller 收尾路径（无信号）也能让服务退出。
        stop_tx.send(()).expect("stop channel must be open");
        handle.await.expect("probe task must not panic")
    }
}
