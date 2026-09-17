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

//! API Key 凭证哈希抽象（`KeyHasher`）与 Argon2id 默认实现。
//!
//! T036 自 `SeaOrmRepository` 抽取：仓储构造注入 `Arc<dyn KeyHasher>`，
//! 不再直接依赖 argon2；哈希算法可独立替换与测试。

use argon2::password_hash::phc::PasswordHash;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};

use crate::core::types::Result;

/// API Key 凭证哈希抽象。
///
/// 签名与被抽取的 `SeaOrmRepository::hash_key` / `verify_key` 实际形态一致：
/// - pepper/salt 语义由实现体持有并统一组装 password 材料，调用方只透传
///   凭证对（`key_id`, `key_secret`）—— 材料编码规则的单一来源随实现体走；
/// - `verify` 对 malformed/空哈希返回 `false` 而非 `Err`：格式坏 = 不予采信，
///   不是系统故障（与既有行为逐字节一致）。
pub trait KeyHasher: Send + Sync {
    /// 哈希一对 API Key 凭证，返回 PHC 格式存储串（约 96 字符，需 VARCHAR(255)）。
    ///
    /// # Errors
    ///
    /// Argon2 参数化/哈希失败时返回 `CoreError::InternalError`。
    fn hash(&self, key_id: &str, key_secret: &str) -> Result<String>;

    /// 校验凭证对是否匹配存储的 PHC 哈希。
    ///
    /// Argon2 的 `verify_password` 内部使用 constant-time 比较，
    /// 等价于原 `subtle::ConstantTimeEq`。
    fn verify(&self, key_id: &str, key_secret: &str, stored_hash: &str) -> bool;
}

/// Argon2id 默认实现（replaces SHA256, CWE-916 fix; OWASP 2023 推荐）。
///
/// - `salt` 作为 pepper（额外加在 password 材料前），增加深度防御；
/// - password-hash 0.6 的 `hash_password` 自动生成 16 字节随机 salt
///   （内嵌 PHC，与旧 SaltString 路径语义等价）。
pub struct Argon2KeyHasher {
    salt: String,
}

impl Argon2KeyHasher {
    /// 以 API key pepper（salt）构造默认哈希器。
    pub fn new(salt: String) -> Self {
        Self { salt }
    }

    /// Argon2 校验的 password 材料：`<pepper>|<key_id>:<key_secret>`。
    ///
    /// 编码规则必须有单一来源：`hash` 与 `verify` 各写一遍时，改分隔符或字段
    /// 顺序只中一处就会让全量已存哈希静默验不过（且编译期无告警）。
    fn password_material(&self, key_id: &str, key_secret: &str) -> String {
        format!("{}|{}:{}", self.salt, key_id, key_secret)
    }
}

impl KeyHasher for Argon2KeyHasher {
    fn hash(&self, key_id: &str, key_secret: &str) -> Result<String> {
        let password = self.password_material(key_id, key_secret);
        let hash = Argon2::default()
            .hash_password(password.as_bytes())
            .map_err(|e| {
                crate::core::CoreError::InternalError(format!("argon2 hash failed: {}", e))
            })?;
        Ok(hash.to_string())
    }

    fn verify(&self, key_id: &str, key_secret: &str, stored_hash: &str) -> bool {
        let parsed = match PasswordHash::new(stored_hash) {
            Ok(h) => h,
            Err(_) => return false,
        };
        let password = self.password_material(key_id, key_secret);
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    }
}
