// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 跨数据中心自动故障转移（cloud-native-dr Phase 3 / T013-T017）。
//!
//! 语义边界：本模块只做**进程内的 DC 择优判定**，不做 etcd 集群间数据复制，也不
//! 承担 Kubernetes 侧的集群外编排（那属 Phase 4 的 operator）。切换之所以可以无损
//! 发生，是因为号段的 dc 位前缀（`datacenter_id_bits`）天然把 worker_id 空间按 DC
//! 分区，两个 DC 不会发出同一个 ID。
//!
//! 判定规则（唯一一条，刻意保持保守）：仅当**当前 active DC 判 `Failed`** 且候选集中
//! 存在 `Healthy` DC 时才切换；`Degraded` 不算可切出（抖动期切走会来回打摆），也不
//! 算可切入（切进一个降级 DC 等于没救活）。没有任何健康候选时返回 `Keep` 并保持原值，
//! 宁可继续带病服务也不制造第二个不确定源。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::EtcdClusterStatus;

/// 数据中心标识，与号段 dc 位（`datacenter_id_bits`）同一值域。
pub type DcId = u8;

/// 各 DC 的健康读视图。`updated_at` 记录最近一次刷新，供调用方判断判据是否已过期。
#[derive(Debug, Clone)]
pub struct DcHealthView {
    per_dc: HashMap<DcId, EtcdClusterStatus>,
    updated_at: Instant,
}

impl DcHealthView {
    /// 空视图：尚未探测过任何 DC。
    #[must_use]
    pub fn new() -> Self {
        Self {
            per_dc: HashMap::new(),
            updated_at: Instant::now(),
        }
    }

    /// 由探测结果构造视图。
    #[must_use]
    pub fn from_statuses(statuses: HashMap<DcId, EtcdClusterStatus>) -> Self {
        Self {
            per_dc: statuses,
            updated_at: Instant::now(),
        }
    }

    #[must_use]
    pub fn status_of(&self, dc: DcId) -> Option<&EtcdClusterStatus> {
        self.per_dc.get(&dc)
    }

    #[must_use]
    pub fn updated_at(&self) -> Instant {
        self.updated_at
    }

    /// 写入/覆盖单个 DC 状态，并刷新时间戳。
    pub fn set(&mut self, dc: DcId, status: EtcdClusterStatus) {
        self.per_dc.insert(dc, status);
        self.updated_at = Instant::now();
    }
}

impl Default for DcHealthView {
    fn default() -> Self {
        Self::new()
    }
}

/// 切换决策。`SwitchTo` 只在判据成立时产生；`Keep` 涵盖「active 仍健康」与
/// 「无可切入候选」两种情形——二者对运维的含义不同，由日志侧按状态明细区分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailoverDecision {
    /// 维持当前 active DC。
    Keep,
    /// 切换到指定 DC（同一次故障只会有一次 `SwitchTo` 生效）。
    SwitchTo(DcId),
}

/// DC 健康探针：注入式，使多集群判定可在无真实 etcd 的环境里被完整测试。
#[async_trait]
pub trait DcProbe: Send + Sync {
    async fn probe(&self, dc: DcId) -> EtcdClusterStatus;
}

/// 跨 DC 故障转移控制器。
///
/// `active` 用原子量而非视图锁承载：`evaluate()` 的并发调用必须只有一个人能把
/// active 换掉（CAS 成功者返回 `SwitchTo`，其余返回 `Keep`），否则多任务并发判
/// 定会重复切换并各自计数，运维侧看到的切换次数与真实拓扑变更次数脱钩。
pub struct DcFailoverController {
    view: Arc<RwLock<DcHealthView>>,
    active: AtomicU8,
    /// 候选 DC 集合（不含 active 自身），升序遍历以保证择优结果确定。
    candidates: Vec<DcId>,
    switches: AtomicU64,
}

impl DcFailoverController {
    /// 以 `primary` 为初始 active DC 构造控制器；`standbys` 为可切入候选集合。
    ///
    /// 传入 primary 自身的重复项会被剔除：把 active 自己当候选会让"切换"退化成
    /// 原地换到自己，产生一次无意义计数并掩盖真实故障。
    #[must_use]
    pub fn new(primary: DcId, mut standbys: Vec<DcId>) -> Self {
        standbys.retain(|dc| *dc != primary);
        standbys.sort_unstable();
        standbys.dedup();
        Self {
            view: Arc::new(RwLock::new(DcHealthView::new())),
            active: AtomicU8::new(primary),
            candidates: standbys,
            switches: AtomicU64::new(0),
        }
    }

    /// 当前 active DC。
    #[must_use]
    pub fn active_dc(&self) -> DcId {
        self.active.load(Ordering::SeqCst)
    }

    /// 累计真实发生的切换次数（并发正确性的观测点）。
    #[must_use]
    pub fn switch_count(&self) -> u64 {
        self.switches.load(Ordering::SeqCst)
    }

    /// 候选 DC 集合（构造时已剔除自身并升序去重）。
    #[must_use]
    pub fn candidates(&self) -> &[DcId] {
        &self.candidates
    }

    /// 共享健康视图句柄：供后台巡检任务写入、供 `/health` 面读取。
    #[must_use]
    pub fn view(&self) -> Arc<RwLock<DcHealthView>> {
        self.view.clone()
    }

    /// 直接写入某 DC 状态（真实 etcd 巡检回路之外的注入点，也用于运维手工置位）。
    pub async fn observe(&self, dc: DcId, status: EtcdClusterStatus) {
        self.view.write().await.set(dc, status);
    }

    /// 用探针批量刷新视图：探测当前 active 与全部候选。
    pub async fn refresh(&self, probe: &dyn DcProbe) {
        let mut seen = Vec::with_capacity(self.candidates.len() + 1);
        seen.push(self.active_dc());
        seen.extend_from_slice(&self.candidates);
        let mut guard = self.view.write().await;
        for dc in seen {
            let status = probe.probe(dc).await;
            guard.set(dc, status);
        }
    }

    /// 判定是否需要切换：active 判 `Failed` 且候选中有 `Healthy` 才切。
    ///
    /// 遍历顺序固定走 `candidates`（构造时已升序去重）而不是 HashMap 迭代：
    /// 后者会让同一现场两次判定给出不同 DC，运维不可复现的切换比不切换更危险。
    pub async fn evaluate(&self) -> FailoverDecision {
        let current = self.active_dc();
        let target = {
            let guard = self.view.read().await;
            if guard.status_of(current) != Some(&EtcdClusterStatus::Failed) {
                // active 未判死（Healthy / Degraded / 未知）一律不切出。
                return FailoverDecision::Keep;
            }
            self.candidates
                .iter()
                .find(|dc| guard.status_of(**dc) == Some(&EtcdClusterStatus::Healthy))
                .copied()
        };
        let Some(target) = target else {
            // 候选里没有一个 Healthy（含降级/未知）：宁可继续带病服务也不切进不确定源。
            return FailoverDecision::Keep;
        };

        // CAS 定胜负：只有把 active 真正换掉的那一次算切换。
        if self
            .active
            .compare_exchange(current, target, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return FailoverDecision::Keep;
        }
        self.switches.fetch_add(1, Ordering::SeqCst);
        tracing::warn!(
            from_dc = current,
            to_dc = target,
            "{}",
            t!(
                "log.core.coordinator.failover.dc_switched",
                from = current,
                to = target
            )
        );
        FailoverDecision::SwitchTo(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 预设状态表的 stub 探针：按 DC 返回固定三态，可计数调用次数。
    struct StubProbe {
        presets: Mutex<HashMap<DcId, EtcdClusterStatus>>,
        calls: AtomicU64,
    }

    impl StubProbe {
        fn new(presets: HashMap<DcId, EtcdClusterStatus>) -> Self {
            Self {
                presets: Mutex::new(presets),
                calls: AtomicU64::new(0),
            }
        }

        fn call_count(&self) -> u64 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DcProbe for StubProbe {
        async fn probe(&self, dc: DcId) -> EtcdClusterStatus {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.presets
                .lock()
                .expect("stub presets lock")
                .get(&dc)
                .cloned()
                .unwrap_or(EtcdClusterStatus::Failed)
        }
    }

    fn statuses(items: &[(DcId, EtcdClusterStatus)]) -> HashMap<DcId, EtcdClusterStatus> {
        items.iter().cloned().collect()
    }

    /// T013：primary 判 Failed 且存在 Healthy standby → 必须 SwitchTo 并改 active。
    #[tokio::test]
    async fn failed_primary_with_healthy_standby_switches() {
        let ctl = DcFailoverController::new(1, vec![2]);
        ctl.observe(1, EtcdClusterStatus::Failed).await;
        ctl.observe(2, EtcdClusterStatus::Healthy).await;

        assert_eq!(ctl.evaluate().await, FailoverDecision::SwitchTo(2));
        assert_eq!(ctl.active_dc(), 2, "切换后 active_dc 必须指向新 DC");
        assert_eq!(ctl.switch_count(), 1);
    }

    /// T013 另一半：primary 仍 Healthy → Keep 且不动 active。
    #[tokio::test]
    async fn healthy_primary_keeps() {
        let ctl = DcFailoverController::new(1, vec![2, 3]);
        ctl.observe(1, EtcdClusterStatus::Healthy).await;
        ctl.observe(2, EtcdClusterStatus::Healthy).await;

        assert_eq!(ctl.evaluate().await, FailoverDecision::Keep);
        assert_eq!(ctl.active_dc(), 1);
        assert_eq!(ctl.switch_count(), 0, "未切换不得计数");
    }

    /// T014：所有 DC 均非 Healthy → Keep 且 active 保持原值（无 standby 不切换）。
    #[tokio::test]
    async fn no_healthy_standby_keeps_original_active_dc() {
        let ctl = DcFailoverController::new(1, vec![2, 3]);
        ctl.observe(1, EtcdClusterStatus::Failed).await;
        // 候选一个降级一个失败：降级不可切入。
        ctl.observe(2, EtcdClusterStatus::Degraded).await;
        ctl.observe(3, EtcdClusterStatus::Failed).await;

        assert_eq!(ctl.evaluate().await, FailoverDecision::Keep);
        assert_eq!(ctl.active_dc(), 1, "无健康候选时必须保持原 active");
        assert_eq!(ctl.switch_count(), 0);
    }

    /// active 仅 Degraded（抖动期）不得切出：否则会在两个 DC 间来回打摆。
    #[tokio::test]
    async fn degraded_primary_does_not_switch_away() {
        let ctl = DcFailoverController::new(1, vec![2]);
        ctl.observe(1, EtcdClusterStatus::Degraded).await;
        ctl.observe(2, EtcdClusterStatus::Healthy).await;

        assert_eq!(ctl.evaluate().await, FailoverDecision::Keep);
        assert_eq!(ctl.active_dc(), 1);
    }

    /// 视图完全未刷新（未知状态）时不得切换：判据缺失不等于判死。
    #[tokio::test]
    async fn unknown_primary_status_keeps() {
        let ctl = DcFailoverController::new(1, vec![2]);
        ctl.observe(2, EtcdClusterStatus::Healthy).await;

        assert_eq!(ctl.evaluate().await, FailoverDecision::Keep);
        assert_eq!(ctl.active_dc(), 1);
    }

    /// 多候选时按 DC 号升序择优，结果必须确定（同一现场不得给出不同目标）。
    #[tokio::test]
    async fn picks_lowest_dc_id_deterministically() {
        let ctl = DcFailoverController::new(0, vec![9, 3, 5]);
        assert_eq!(ctl.candidates(), &[3, 5, 9], "候选必须升序去重");
        for dc in [3, 5, 9] {
            ctl.observe(dc, EtcdClusterStatus::Healthy).await;
        }
        ctl.observe(0, EtcdClusterStatus::Failed).await;

        assert_eq!(ctl.evaluate().await, FailoverDecision::SwitchTo(3));
    }

    /// 把 active 自身混进候选必须被剔除：否则会出现"切到自己"的空转计数。
    #[tokio::test]
    async fn constructor_drops_self_from_candidates() {
        let ctl = DcFailoverController::new(7, vec![7, 8, 7]);
        assert_eq!(ctl.candidates(), &[8]);
    }

    /// T016：`DcProbe` 注入 stub 后 `refresh` 必须把 active 与全部候选都写进视图。
    #[tokio::test]
    async fn refresh_probes_active_and_all_candidates() {
        let ctl = DcFailoverController::new(1, vec![2, 3]);
        let probe = StubProbe::new(statuses(&[
            (1, EtcdClusterStatus::Healthy),
            (2, EtcdClusterStatus::Degraded),
            // DC 3 不在预设表内：stub 回 Failed，验证"缺项即不健康"。
        ]));

        ctl.refresh(&probe).await;

        assert_eq!(probe.call_count(), 3, "必须探测 active + 两个候选");
        let view_handle = ctl.view();
        let view = view_handle.read().await;
        assert_eq!(view.status_of(1), Some(&EtcdClusterStatus::Healthy));
        assert_eq!(view.status_of(2), Some(&EtcdClusterStatus::Degraded));
        assert_eq!(view.status_of(3), Some(&EtcdClusterStatus::Failed));
    }

    /// refresh 到 Failed + Healthy 后，同一路径必须直接可切换（探针与判定闭环）。
    #[tokio::test]
    async fn refresh_then_evaluate_switches_via_probe_results() {
        let ctl = DcFailoverController::new(4, vec![5]);
        let probe = StubProbe::new(statuses(&[
            (4, EtcdClusterStatus::Failed),
            (5, EtcdClusterStatus::Healthy),
        ]));

        ctl.refresh(&probe).await;
        assert_eq!(ctl.evaluate().await, FailoverDecision::SwitchTo(5));
        assert_eq!(ctl.active_dc(), 5);
    }

    /// T019 的并发正确性部分：16 线程并发 evaluate 只允许产生一次 SwitchTo。
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_evaluates_produce_exactly_one_switch() {
        let ctl = Arc::new(DcFailoverController::new(1, vec![2]));
        ctl.observe(1, EtcdClusterStatus::Failed).await;
        ctl.observe(2, EtcdClusterStatus::Healthy).await;

        let mut handles = Vec::new();
        for _ in 0..16 {
            let ctl = ctl.clone();
            handles.push(tokio::spawn(async move { ctl.evaluate().await }));
        }
        let mut decisions = Vec::with_capacity(handles.len());
        for handle in handles {
            decisions.push(handle.await.expect("判定任务不得 panic"));
        }

        let switches = decisions
            .iter()
            .filter(|d| matches!(d, FailoverDecision::SwitchTo(_)))
            .count();
        assert_eq!(
            switches, 1,
            "并发判定必须只放行一次切换，实际 {switches} 次"
        );
        assert_eq!(ctl.switch_count(), 1);
        assert_eq!(ctl.active_dc(), 2);

        // 切换完成后再次判定：active 已是健康 DC，不得继续切换。
        assert_eq!(ctl.evaluate().await, FailoverDecision::Keep);
        assert_eq!(ctl.switch_count(), 1, "Keep 不得增加计数");
    }

    /// `DcHealthView` 契约：空视图无状态、set 刷新时间戳且不引入未知 DC。
    #[tokio::test]
    async fn view_contract_for_empty_and_written_states() {
        let mut view = DcHealthView::new();
        let before = view.updated_at();
        assert_eq!(view.status_of(1), None);

        view.set(1, EtcdClusterStatus::Healthy);
        assert_eq!(view.status_of(1), Some(&EtcdClusterStatus::Healthy));
        assert!(view.updated_at() >= before, "写入必须刷新 updated_at");
        assert_eq!(view.status_of(9), None, "未写入的 DC 不得凭空出现状态");

        let built = DcHealthView::from_statuses(statuses(&[(3, EtcdClusterStatus::Degraded)]));
        assert_eq!(built.status_of(3), Some(&EtcdClusterStatus::Degraded));
    }
}
