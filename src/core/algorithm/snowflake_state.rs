// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 雪花算法重启时钟守卫的本地状态文件。
//!
//! 进程运行期间周期性持久化「最后发号毫秒时间戳」；启动时若发现
//! `last_issued_ts` 晚于当前墙钟（跨重启时钟回拨），守卫方必须拒绝启动，
//! 否则重启后新发 ID 与回拨前的历史 ID 撞车。文件按 `dc + worker` 命名，
//! 文件名不含 pid——重启后同 worker 要能读回上一次的进度（对比 etcd
//! cache 的 `{dc}_{pid}` 命名，那份缓存恰恰不能跨重启复用）。
//!
//! 写入为 tmp + rename 原子写，进程崩溃不产生半写文件；解析失败按
//! 「无状态」处理（返回 `None` 并告警），首次启动/文件损坏不阻断启动。

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 本地守卫状态文件的持久化内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnowflakeStateFile {
    /// 最后一次发号的毫秒时间戳（自 epoch 起的 Unix 毫秒，与
    /// `SnowflakeAlgorithm::get_timestamp` 同口径）。
    pub last_issued_ts: u64,
    pub dc_id: u8,
    pub worker_id: u8,
    /// 保存时刻（诊断用，ISO-8601 字符串）。
    pub saved_at: String,
}

/// 状态文件约定路径：`<dir>/snowflake_state_{dc}_{worker}.json`。
pub fn state_file_path(dir: &Path, dc_id: u8, worker_id: u8) -> PathBuf {
    dir.join(format!("snowflake_state_{}_{}.json", dc_id, worker_id))
}

/// 读取状态文件。文件不存在或内容损坏（解析失败）一律返回 `None`——
/// 调用方把 `None` 视为「无历史」，不阻断启动；损坏场景在此处告警。
pub fn load(path: &Path) -> Option<SnowflakeStateFile> {
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(
                event = "snowflake_state_read_failed",
                path = %path.display(),
                error = %e
            );
            return None;
        }
    };

    match serde_json::from_slice::<SnowflakeStateFile>(&raw) {
        Ok(state) => Some(state),
        Err(e) => {
            tracing::warn!(
                event = "snowflake_state_corrupt",
                path = %path.display(),
                error = %e
            );
            None
        }
    }
}

/// 原子写状态文件：先写同目录 tmp 文件再 rename 覆盖，目录不存在则创建。
pub fn save(path: &Path, state: &SnowflakeStateFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        let json = serde_json::to_vec_pretty(state)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        file.write_all(&json)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nebulaid-snowflake-state-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample(last_issued_ts: u64) -> SnowflakeStateFile {
        SnowflakeStateFile {
            last_issued_ts,
            dc_id: 1,
            worker_id: 2,
            saved_at: "2026-09-24T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let path = state_file_path(&dir, 1, 2);
        save(&path, &sample(123_456_789)).expect("save must succeed");

        let loaded = load(&path).expect("loaded state must exist");
        assert_eq!(loaded.last_issued_ts, 123_456_789);
        assert_eq!(loaded.dc_id, 1);
        assert_eq!(loaded.worker_id, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_missing_file_returns_none() {
        let dir = temp_dir("missing");
        let path = state_file_path(&dir, 9, 9);
        assert!(load(&path).is_none(), "missing file must yield None");
    }

    #[test]
    fn test_load_corrupt_file_returns_none() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = state_file_path(&dir, 3, 4);
        std::fs::write(&path, b"{ not valid json").unwrap();
        assert!(load(&path).is_none(), "corrupt file must yield None");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_creates_missing_parent_dirs() {
        let dir = temp_dir("mkdirs");
        let path = dir.join("nested").join("deep").join("state.json");
        save(&path, &sample(1)).expect("save must create parent dirs");
        assert!(load(&path).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
