// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! NebulaIdCluster reconcile 回路：为每个 DcSpec ensure StatefulSet + Headless Service，
//! 周期探测各 DC 的 `/health`，回写 CR status 并按与主 crate failover 同款保守语义
//! 维护 active_dc。本 crate 不依赖主 crate（保持 operator 镜像轻量），择优语义的
//! 漂移由 parity 单测钉住。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::{Container, Service, ServicePort, ServiceSpec};
use k8s_openapi::api::core::v1::{PodSpec, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta, OwnerReference};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Client, ResourceExt};

/// CRD API 版本常量（类型为本 crate 自有，与 crd.rs 的 kube 声明一致）。
const NEBULA_ID_CLUSTER_API_VERSION: &str = "nebulaid.kirky.x/v1alpha1";
use thiserror::Error;

use crate::crd::{DcSpec, DcStatus, NebulaIdCluster, NebulaIdClusterStatus};

/// SSA field manager：operator 写下的字段都打上此归属，便于 `kubectl diff` 与交接。
pub const FIELD_MANAGER: &str = "nebulaid-operator";

/// NebulaId 容器端口与 Headless Service 端口（operator 管理面契约）。
pub const NEBULAID_HTTP_PORT: i32 = 8080;

#[derive(Debug, Error)]
pub enum OperatorError {
    #[error("kube api error: {0}")]
    Kube(#[from] kube::Error),
    #[error("cluster object has no name")]
    UnnamedCluster,
}

/// reconcile 回路的共享上下文：kube 客户端与 `/health` 探测用的 HTTP 客户端。
pub struct ControllerContext {
    pub client: Client,
    pub http: reqwest::Client,
    /// `/health` 探测超时；超时按不健康判定（readiness 不得被挂死的实例拖住）。
    pub probe_timeout: Duration,
}

/// 生成 NebulaId 资源名的统一入口：`nebulaid-{cluster}-dc{id}`。
pub fn resource_name(cluster_name: &str, dc_id: u8) -> String {
    format!("nebulaid-{cluster_name}-dc{dc_id}")
}

/// DC 的 StatefulSet/Service 公共标签集（selector 与模板必须一致）。
fn dc_labels(cluster_name: &str, dc_id: u8) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("app.kubernetes.io/name".to_string(), "nebulaid".to_string()),
        (
            "app.kubernetes.io/managed-by".to_string(),
            FIELD_MANAGER.to_string(),
        ),
        (
            "nebulaid.kirky.x/cluster".to_string(),
            cluster_name.to_string(),
        ),
        ("nebulaid.kirky.x/dc".to_string(), dc_id.to_string()),
    ])
}

/// CR 的 owner reference（controller 角色指向 CR 本体）：CR 删除时连带回收
/// operator 创建的 StatefulSet/Service，不留孤儿资源。
fn owner_reference(cluster: &NebulaIdCluster) -> Option<OwnerReference> {
    let uid = cluster.metadata.uid.clone()?;
    Some(OwnerReference {
        api_version: NEBULA_ID_CLUSTER_API_VERSION.to_string(),
        kind: "NebulaIdCluster".to_string(),
        name: cluster.name_any(),
        uid,
        controller: Some(true),
        block_owner_deletion: Some(true),
    })
}

/// 期望的 StatefulSet：镜像版本来自 `spec.version`，etcd 端点与 dc 位经环境变量
/// 注入容器（`NEBULA_ETCD_ENDPOINTS` / `NEBULA_DC_ID`），etcd 端点随 CR 更新而滚动。
pub fn desired_stateful_set(
    cluster: &NebulaIdCluster,
    dc: &DcSpec,
) -> Result<StatefulSet, OperatorError> {
    let cluster_name = if cluster.metadata.name.is_some() {
        cluster.name_any()
    } else {
        return Err(OperatorError::UnnamedCluster);
    };
    let labels = dc_labels(&cluster_name, dc.id);
    let image = format!("nebulaid:{}", cluster.spec.version);
    let etcd_endpoints = dc.etcd_endpoints.join(",");

    Ok(StatefulSet {
        metadata: ObjectMeta {
            name: Some(resource_name(&cluster_name, dc.id)),
            labels: Some(labels.clone()),
            owner_references: owner_reference(cluster).map(|r| vec![r]),
            ..Default::default()
        },
        spec: Some(k8s_openapi::api::apps::v1::StatefulSetSpec {
            replicas: Some(cluster.spec.replicas as i32),
            // Headless Service 名与 STS 同名：稳态网络标识 + podDNS 域名可预期。
            service_name: Some(resource_name(&cluster_name, dc.id)),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "nebulaid".to_string(),
                        image: Some(image),
                        env: Some(vec![
                            k8s_openapi::api::core::v1::EnvVar {
                                name: "NEBULA_ETCD_ENDPOINTS".to_string(),
                                value: Some(etcd_endpoints),
                                ..Default::default()
                            },
                            k8s_openapi::api::core::v1::EnvVar {
                                name: "NEBULA_DC_ID".to_string(),
                                value: Some(dc.id.to_string()),
                                ..Default::default()
                            },
                        ]),
                        ports: Some(vec![k8s_openapi::api::core::v1::ContainerPort {
                            container_port: NEBULAID_HTTP_PORT,
                            name: Some("http".to_string()),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    })
}

/// 期望的 Headless Service（`clusterIP: None`）：端点发现走 DNS，不经过虚拟 IP。
pub fn desired_headless_service(
    cluster: &NebulaIdCluster,
    dc: &DcSpec,
) -> Result<Service, OperatorError> {
    let cluster_name = if cluster.metadata.name.is_some() {
        cluster.name_any()
    } else {
        return Err(OperatorError::UnnamedCluster);
    };
    let labels = dc_labels(&cluster_name, dc.id);

    Ok(Service {
        metadata: ObjectMeta {
            name: Some(resource_name(&cluster_name, dc.id)),
            labels: Some(labels.clone()),
            owner_references: owner_reference(cluster).map(|r| vec![r]),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            cluster_ip: Some("None".to_string()),
            selector: Some(labels),
            ports: Some(vec![ServicePort {
                name: Some("http".to_string()),
                port: NEBULAID_HTTP_PORT,
                target_port: Some(IntOrString::String("http".to_string())),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    })
}

/// 单个 DC 的 `/health` 探测：HTTP 200 即健康，其余（非 2xx / 网络错误 / 超时）不健康。
pub async fn probe_dc_health(http: &reqwest::Client, url: &str, timeout: Duration) -> bool {
    match http.get(url).timeout(timeout).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// DC 健康探测结果的择优判定（与主 crate `DcFailoverController` 保守语义对齐）：
/// 仅当当前 active 明确不健康且存在健康候选才切到**dc 号最小**的健康候选；
/// 当前缺失（未知）、健康、或无健康候选时一律保持原值——判据缺失不等于判死，
/// 降级期切走会在两个 DC 间来回打摆。
pub fn decide_active_dc(current: u8, dc_health: &[(u8, bool)]) -> u8 {
    let current_healthy = dc_health
        .iter()
        .find(|(dc, _)| *dc == current)
        .map(|(_, healthy)| *healthy);
    match current_healthy {
        Some(false) => dc_health
            .iter()
            .filter(|(dc, healthy)| *healthy && *dc != current)
            .map(|(dc, _)| *dc)
            .min()
            .unwrap_or(current),
        // None（未探测到）与 Some(true) 都保持现值。
        _ => current,
    }
}

/// 由探测结果构造要回写的 status：phase 按 active_dc 的健康度与候选健康数汇总。
pub fn build_cluster_status(
    cluster: &NebulaIdCluster,
    dc_health: &[(u8, bool)],
) -> NebulaIdClusterStatus {
    let per_dc_health = dc_health
        .iter()
        .map(|(id, healthy)| DcStatus {
            id: *id,
            healthy: *healthy,
        })
        .collect();
    // 初始 active 取 spec 首个 DC（与主 crate DcFailoverController::new(primary, _) 同构）；
    // 已有 status 则沿用上次回写的 active_dc，保证择优只在实际判死时发生。
    let current = cluster
        .status
        .as_ref()
        .map(|s| s.active_dc)
        .unwrap_or_else(|| {
            cluster
                .spec
                .datacenters
                .first()
                .map(|dc| dc.id)
                .unwrap_or(0)
        });
    let active_dc = decide_active_dc(current, dc_health);
    let active_healthy = dc_health
        .iter()
        .find(|(dc, _)| *dc == active_dc)
        .map(|(_, healthy)| *healthy)
        .unwrap_or(false);
    let phase = if active_healthy && dc_health.iter().all(|(_, h)| *h) {
        "Ready"
    } else if active_healthy {
        "Degraded"
    } else {
        "Pending"
    };
    NebulaIdClusterStatus {
        phase: phase.to_string(),
        active_dc,
        per_dc_health,
        endpoints: cluster
            .spec
            .datacenters
            .iter()
            .map(|dc| {
                format!(
                    "http://{}.{}.svc:{NEBULAID_HTTP_PORT}",
                    resource_name(&cluster.name_any(), dc.id),
                    cluster.namespace().unwrap_or_else(|| "default".into())
                )
            })
            .collect(),
    }
}

/// 单簇 reconcile：ensure 每个_dc 的 StatefulSet + Headless Service（SSA apply），
/// 探测 `/health` 后回写 status。返回 requeue 周期（探测与收敛都是周期性的）。
pub async fn reconcile(
    cluster: Arc<NebulaIdCluster>,
    ctx: Arc<ControllerContext>,
) -> Result<Action, OperatorError> {
    let namespace = cluster.namespace().unwrap_or_else(|| "default".to_string());
    let dc_count = cluster.spec.datacenters.len();

    let stateful_sets: Api<StatefulSet> = Api::namespaced(ctx.client.clone(), &namespace);
    let services: Api<Service> = Api::namespaced(ctx.client.clone(), &namespace);
    let patch_params = PatchParams::apply(FIELD_MANAGER).force();

    for dc in &cluster.spec.datacenters {
        let sts = desired_stateful_set(&cluster, dc)?;
        stateful_sets
            .patch(
                sts.metadata.name.as_deref().unwrap_or_default(),
                &patch_params,
                &Patch::Apply(&sts),
            )
            .await?;
        let svc = desired_headless_service(&cluster, dc)?;
        services
            .patch(
                svc.metadata.name.as_deref().unwrap_or_default(),
                &patch_params,
                &Patch::Apply(&svc),
            )
            .await?;
    }

    // 周期 `/health` 探测（requeue 即周期）：逐 DC 探测 → 择优 → 回写 status。
    let mut dc_health = Vec::with_capacity(dc_count);
    for dc in &cluster.spec.datacenters {
        let url = format!(
            "http://{}.{}.svc:{NEBULAID_HTTP_PORT}/health",
            resource_name(&cluster.name_any(), dc.id),
            namespace
        );
        let healthy = probe_dc_health(&ctx.http, &url, ctx.probe_timeout).await;
        dc_health.push((dc.id, healthy));
    }
    let status = build_cluster_status(&cluster, &dc_health);

    let clusters: Api<NebulaIdCluster> = Api::namespaced(ctx.client.clone(), &namespace);
    clusters
        .patch_status(
            &cluster.name_any(),
            &PatchParams::apply(FIELD_MANAGER).force(),
            &Patch::Merge(serde_json::json!({ "status": status })),
        )
        .await?;

    Ok(Action::requeue(Duration::from_secs(30)))
}

/// reconcile 失败的退避策略：固定 10s 重试（错误已由运行时逐条记录，不指数放大）。
pub fn error_policy(
    _cluster: Arc<NebulaIdCluster>,
    error: &OperatorError,
    _ctx: Arc<ControllerContext>,
) -> Action {
    tracing::warn!(error = %error, "reconcile failed; requeueing in 10s");
    Action::requeue(Duration::from_secs(10))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::NebulaIdClusterSpec;

    fn two_dc_cluster() -> NebulaIdCluster {
        let mut cluster = NebulaIdCluster::new(
            "prod",
            NebulaIdClusterSpec {
                replicas: 3,
                datacenters: vec![
                    DcSpec {
                        id: 1,
                        etcd_endpoints: vec!["http://etcd-a:2379".to_string()],
                    },
                    DcSpec {
                        id: 2,
                        etcd_endpoints: vec!["http://etcd-b:2379".to_string()],
                    },
                ],
                version: "0.3.0-rc.4".to_string(),
            },
        );
        cluster.metadata.uid = Some("uid-prod-1".to_string());
        cluster
    }

    /// 2 个 DcSpec 必须生成 2 组 STS+Svc：名称按 dc 区分、replicas 继承 spec、
    /// Service 为 headless（clusterIP=None）、owner reference 指回 CR。
    #[test]
    fn two_dc_specs_produce_two_stateful_sets_and_services() {
        let cluster = two_dc_cluster();

        let mut sts_names = Vec::new();
        let mut svc_names = Vec::new();
        for dc in &cluster.spec.datacenters {
            let sts = desired_stateful_set(&cluster, dc).expect("STS 构造必须成功");
            let svc = desired_headless_service(&cluster, dc).expect("Service 构造必须成功");

            assert_eq!(sts.spec.as_ref().unwrap().replicas, Some(3));
            assert_eq!(
                sts.spec.as_ref().unwrap().service_name,
                Some(svc.metadata.name.clone().unwrap()),
                "STS 必须挂同名 Headless Service"
            );
            // etcd 端点与 dc 位经环境变量注入容器。
            let container = &sts
                .spec
                .as_ref()
                .unwrap()
                .template
                .spec
                .as_ref()
                .unwrap()
                .containers[0];
            let env_names: Vec<&str> = container
                .env
                .as_ref()
                .unwrap()
                .iter()
                .map(|e| e.name.as_str())
                .collect();
            assert!(env_names.contains(&"NEBULA_ETCD_ENDPOINTS"));
            assert!(env_names.contains(&"NEBULA_DC_ID"));

            assert_eq!(
                svc.spec.as_ref().unwrap().cluster_ip,
                Some("None".to_string()),
                "端点发现 Service 必须为 headless"
            );

            // owner reference 指回 CR（controller 角色），CR 删除即连带回收。
            let owner = &sts.metadata.owner_references.as_ref().unwrap()[0];
            assert_eq!(owner.uid, "uid-prod-1");
            assert_eq!(owner.kind, "NebulaIdCluster");
            assert_eq!(owner.controller, Some(true));

            sts_names.push(sts.metadata.name.unwrap());
            svc_names.push(svc.metadata.name.unwrap());
        }

        assert_eq!(sts_names.len(), 2, "2 个 DcSpec 必须生成 2 个 StatefulSet");
        assert_eq!(svc_names.len(), 2, "2 个 DcSpec 必须生成 2 个 Service");
        assert!(sts_names.contains(&"nebulaid-prod-dc1".to_string()));
        assert!(sts_names.contains(&"nebulaid-prod-dc2".to_string()));
        assert_eq!(sts_names, svc_names, "STS 与 Service 成对同名");
    }

    /// 择优语义与主 crate DcFailoverController parity：
    /// active 判死 + 健康候选 → 切最小 dc 号；active 健康 / 未知 / 无健康候选 → 保持。
    #[test]
    fn decide_active_dc_parity_with_in_process_failover_semantics() {
        // primary Failed + standby Healthy → SwitchTo。
        assert_eq!(decide_active_dc(1, &[(1, false), (2, true)]), 2);
        // primary Healthy → Keep。
        assert_eq!(decide_active_dc(1, &[(1, true), (2, true)]), 1);
        // 无健康候选 → Keep（宁可带病服务也不切进不确定源）。
        assert_eq!(decide_active_dc(1, &[(1, false), (2, false)]), 1);
        // active 未出现在探测结果（未知）→ Keep（判据缺失不等于判死）。
        assert_eq!(decide_active_dc(1, &[(2, true)]), 1);
        // 多候选按 dc 号升序择优（确定性）。
        assert_eq!(decide_active_dc(0, &[(0, false), (9, true), (3, true)]), 3);
        // 切换候选不含 active 自身。
        assert_eq!(decide_active_dc(3, &[(3, false)]), 3);
    }

    /// 回写的 status patch：active_dc 择优落位、per_dc_health 逐 DC 记录、
    /// endpoints 按 DC 展开为集群内 DNS。
    #[test]
    fn build_cluster_status_aggregates_health_and_endpoints() {
        let mut cluster = two_dc_cluster();
        // 演练「已确立的 active 判死」：先回写 status.active_dc=1，再以探测结果触发择优。
        cluster.status = Some(NebulaIdClusterStatus {
            active_dc: 1,
            ..Default::default()
        });
        let status = build_cluster_status(&cluster, &[(1, false), (2, true)]);

        assert_eq!(status.active_dc, 2, "active 判死必须择优切到 dc=2");
        assert_eq!(status.phase, "Degraded", "active 健康但有 DC 不健康");
        assert_eq!(status.per_dc_health.len(), 2);
        assert_eq!(status.endpoints.len(), 2);
        assert!(status.endpoints[0].contains("nebulaid-prod-dc1.default.svc:8080"));

        let all_healthy = build_cluster_status(&cluster, &[(1, true), (2, true)]);
        assert_eq!(all_healthy.phase, "Ready", "全部健康为 Ready");
        assert_eq!(
            all_healthy.active_dc, 1,
            "无择优触发时保持初始 active（spec 首个 DC）"
        );

        // status 可直接作为 Merge patch 的 JSON 载荷（回写路径的形状契约）。
        let patch = serde_json::json!({ "status": status });
        assert!(patch["status"]["activeDc"].is_u64());
    }
}
