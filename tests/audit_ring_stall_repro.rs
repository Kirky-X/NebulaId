// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use nebulaid::server::audit::{AuditEvent, AuditEventType, AuditLogger, AuditResult};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn ev(i: u64) -> AuditEvent {
    AuditEvent::new(
        AuditEventType::IdGeneration,
        None,
        format!("GET /health#{i}"),
        "/health".to_string(),
        AuditResult::Success,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_logging_stays_fast_after_ring_and_channel_saturation() {
    // 与 src/main.rs init_observability 相同的订阅者配置（含 file sink 排水）。
    let sink_path = std::env::temp_dir().join(format!(
        "nebulaid-audit-regression-{}.log",
        std::process::id()
    ));
    let _logger = inklog::LoggerManager::builder()
        .level("info")
        .format("{timestamp} [{level}] {target} - {message}")
        .console_json(true)
        .file(&sink_path)
        .build()
        .await
        .expect("inklog LoggerManager must build");

    let audit = AuditLogger::new(100);

    // 未满基线：100 条应远低于阻塞阈值。
    let start = Instant::now();
    for i in 0..100 {
        audit.log(ev(i)).await;
    }
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "audit logging must stay fast before saturation, took {:?}",
        start.elapsed()
    );

    // 灌满 inklog async 通道（默认容量 10000）+ 审计环（100）：越过容量后
    // 若 send_timeout 阻塞回归，下一段的断言会失败。
    for i in 0..10_500u64 {
        audit.log(ev(i)).await;
    }

    // 容量饱和后的 log() 必须仍然快（每次耗时以 20ms 为上限，远宽于
    // 修复后的微秒级、远紧于回归时的 ~100ms 阻塞）。
    // llvm-cov 插桩（CI default leg 即插桩运行本测试）会把绝对时长整体拖高
    // 数倍，20ms 恒定上限在插桩下偶发误报；检测到 LLVM_PROFILE_FILE（llvm-cov
    // 注入）时放宽到 80ms——仍在回归信号（~100ms 阻塞）之下，保持区分度。
    let instrumented = std::env::var_os("LLVM_PROFILE_FILE").is_some();
    let per_call_cap = if instrumented {
        Duration::from_millis(80)
    } else {
        Duration::from_millis(20)
    };
    let mut worst = Duration::ZERO;
    for i in 10_500..10_520u64 {
        let start = Instant::now();
        audit.log(ev(i)).await;
        worst = worst.max(start.elapsed());
    }
    assert!(
        worst < per_call_cap,
        "audit log() must not block after channel saturation (worst={worst:?}); \
         this indicates the inklog async channel is undrained or the audit mutex \
         spans a blocking log call"
    );

    // 64 并发 × 10 次灌压后同样断言（覆盖并发排队形态）；插桩下同样按比例
    // 放宽（见上），真回归（每次 ~100ms 阻塞在锁上）仍远超上限。
    let audit = Arc::new(audit);
    let mut handles = Vec::new();
    for j in 0..64u64 {
        let a = Arc::clone(&audit);
        handles.push(tokio::spawn(async move {
            let mut local_worst = Duration::ZERO;
            for i in 0..10u64 {
                let start = Instant::now();
                a.log(ev(20_000 + j * 10 + i)).await;
                local_worst = local_worst.max(start.elapsed());
            }
            local_worst
        }));
    }
    for h in handles {
        worst = worst.max(h.await.unwrap());
    }
    let concurrent_cap = if instrumented {
        Duration::from_millis(300)
    } else {
        Duration::from_millis(100)
    };
    assert!(
        worst < concurrent_cap,
        "concurrent audit log() must not serialize on a blocking log backend \
         (worst={worst:?})"
    );

    let _ = std::fs::remove_file(&sink_path);
}
