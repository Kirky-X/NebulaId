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

//! # 仓库文本守卫（converge T020）
//!
//! 本 crate 不存在可用的「全特性」构建：dbnexus 禁止 `sqlite` 与 `postgres`
//! 同时启用（`compile_error!`），而 default 特性集恒含 `dbnexus/postgres`。
//! CI 与文档若继续推荐该开关，等于宣称一个必然失败的门禁，故用测试钉住。
//!
//! 判定口径（显式规则，不做语义猜测）：被扫描文件**同一行**同时出现 `cargo`
//! 与全特性开关，视为「对外推荐该命令」→ 失败；仅出现在禁止性说明里的字面量
//! （该行不含 `cargo`）允许保留，以便解释为何不能用该开关。

use std::fs;
use std::path::{Path, PathBuf};

/// 全特性开关字面量。拆分拼接，避免本文件被同类文本检索守卫自匹配。
fn all_features_flag() -> String {
    format!("--all{}features", "-")
}

/// 扫描范围：面向用户的 README、CI 配置目录、全部文档目录。
fn scanned_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for name in ["README.md", "README_zh.md"] {
        let path = root.join(name);
        if path.is_file() {
            files.push(path);
        }
    }
    for dir in [".github", "docs"] {
        collect_dir(&root.join(dir), &mut files);
    }
    files
}

fn collect_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dir(&path, out);
            continue;
        }
        let ext_ok = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md") | Some("yml") | Some("yaml")
        );
        if ext_ok {
            out.push(path);
        }
    }
}

#[test]
fn ci_and_docs_never_recommend_the_unbuildable_feature_switch() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let flag = all_features_flag();
    let mut violations = Vec::new();
    let mut scanned = 0usize;

    for path in scanned_files(root) {
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                violations.push(format!("{} 读取失败: {e}", path.display()));
                continue;
            }
        };
        scanned += 1;
        for (idx, line) in text.lines().enumerate() {
            if line.contains("cargo") && line.contains(&flag) {
                violations.push(format!("{}:{}: {}", path.display(), idx + 1, line.trim()));
            }
        }
    }

    // 守卫自身必须真的扫到内容，否则等于空转通过
    assert!(scanned >= 10, "扫描文件数异常（{scanned}），守卫可能失效");
    assert!(
        violations.is_empty(),
        "以下位置仍在推荐不可构建的全特性命令（请改用 --features etcd）：\n{}",
        violations.join("\n")
    );
}
