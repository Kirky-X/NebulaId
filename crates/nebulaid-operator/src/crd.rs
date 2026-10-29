// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! `NebulaIdCluster` 自定义资源定义（kube-rs CustomResource 派生）。
//!
//! spec 描述期望的 NebulaId 部署拓扑（副本数、数据中心与各自的 etcd 端点、镜像版本）；
//! status 由 operator 回写（整体 phase、active_dc 与各 DC 健康明细、服务端点）。

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 期望的 NebulaId 集群部署拓扑。
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, Default, JsonSchema, PartialEq)]
#[kube(
    group = "nebulaid.kirky.x",
    version = "v1alpha1",
    kind = "NebulaIdCluster",
    namespaced,
    status = "NebulaIdClusterStatus",
    derive = "Default",
    doc = "NebulaId 集群编排：per-DC StatefulSet/Service 与跨 DC 故障转移状态管理"
)]
#[serde(rename_all = "camelCase")]
pub struct NebulaIdClusterSpec {
    /// 每个 DC 的 NebulaId 副本数（StatefulSet replicas）。
    pub replicas: u16,
    /// 数据中心清单：dc 位标识与该 DC 的 etcd 端点。
    pub datacenters: Vec<DcSpec>,
    /// NebulaId 镜像版本（StatefulSet image tag）。
    pub version: String,
}

/// 单个数据中心的部署参数。`id` 即号段 dc 位前缀（DcId 值域 u8），
/// 跨 DC 命名空间经 dc 位分区天然不重叠。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DcSpec {
    pub id: u8,
    pub etcd_endpoints: Vec<String>,
}

/// operator 回写的集群状态。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NebulaIdClusterStatus {
    /// 集群整体阶段：`Pending` / `Ready` / `Degraded`。
    pub phase: String,
    /// 当前承载发号流量的 DC（与主 crate failover 语义对齐的择优结果）。
    pub active_dc: u8,
    /// 各 DC 健康明细。
    pub per_dc_health: Vec<DcStatus>,
    /// 可供客户端接入的 NebulaId 服务端点。
    pub endpoints: Vec<String>,
}

/// 单个 DC 的健康快照。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DcStatus {
    pub id: u8,
    /// `/health` 探测结果：true = 可达且健康。
    pub healthy: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CRD 序列化往返：status 字段相等（spec/status 全量 round-trip）。
    #[test]
    fn nebula_id_cluster_serde_round_trip_preserves_status() {
        let cluster = NebulaIdCluster::new(
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
                        etcd_endpoints: vec![
                            "http://etcd-b-0:2379".to_string(),
                            "http://etcd-b-1:2379".to_string(),
                        ],
                    },
                ],
                version: "0.3.0-rc.4".to_string(),
            },
        );

        let raw = serde_json::to_string(&cluster).expect("序列化必须成功");
        let parsed: NebulaIdCluster = serde_json::from_str(&raw).expect("反序列化必须成功");

        assert_eq!(parsed.spec, cluster.spec, "spec 往返必须相等");
        assert!(parsed.status.is_none(), "未回写 status 时保持 None");

        // 带 status 的完整对象往返：status 字段逐项相等。
        let mut with_status = cluster;
        with_status.status = Some(NebulaIdClusterStatus {
            phase: "Degraded".to_string(),
            active_dc: 2,
            per_dc_health: vec![
                DcStatus {
                    id: 1,
                    healthy: false,
                },
                DcStatus {
                    id: 2,
                    healthy: true,
                },
            ],
            endpoints: vec!["http://nebulaid-prod-dc2.default.svc:8080".to_string()],
        });

        let raw = serde_json::to_string(&with_status).expect("序列化必须成功");
        let parsed: NebulaIdCluster = serde_json::from_str(&raw).expect("反序列化必须成功");
        assert_eq!(parsed.status, with_status.status, "status 往返必须相等");

        // 生成的 CRD 字段名走 camelCase（K8s 惯例），status 顶层键存在。
        let value: serde_json::Value = serde_json::from_str(&raw).expect("JSON 解析必须成功");
        assert_eq!(
            value["spec"]["replicas"], 3,
            "replicas 必须映射 spec.replicas"
        );
        assert_eq!(
            value["spec"]["datacenters"][0]["etcdEndpoints"][0], "http://etcd-a:2379",
            "etcdEndpoints 必须为 camelCase 键（数组元素可寻址）"
        );
        assert_eq!(
            value["status"]["activeDc"], 2,
            "activeDc 必须为 camelCase 键"
        );
    }
}
