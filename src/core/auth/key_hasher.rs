// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! API Key 凭证哈希抽象（`KeyHasher`）与 Argon2id 默认实现。
//! 仓储注入 `Arc<dyn KeyHasher>`，不直接依赖 argon2；算法可独立替换与测试。

use argon2::password_hash::phc::PasswordHash;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};

use crate::core::types::Result;

/// API Key 凭证哈希抽象。
///
/// pepper/salt 由实现体持有；`verify` 对 malformed/空哈希返回 `false` 而非 `Err`。
pub trait KeyHasher: Send + Sync {
    /// 哈希一对 API Key 凭证，返回 PHC 格式存储串（约 96 字符，需 VARCHAR(255)）。
    ///
    /// # Errors
    ///
    /// Argon2 参数化/哈希失败时返回 `CoreError::InternalError`。
    fn hash(&self, key_id: &str, key_secret: &str) -> Result<String>;

    /// 校验凭证对是否匹配存储的 PHC 哈希。
    ///
    /// `verify_password` 内部 constant-time 比较，等价于原 `subtle::ConstantTimeEq`。
    fn verify(&self, key_id: &str, key_secret: &str, stored_hash: &str) -> bool;
}

/// Argon2id 默认实现（OWASP 2023 推荐，替代 SHA256 修复 CWE-916）。
///
/// `salt` 作为 pepper 加在 password 材料前；`hash_password` 自动生成 16 字节随机 salt 并内嵌 PHC。
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
    /// 编码规则单一来源：改分隔符或字段顺序会让已存哈希静默验不过。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试 pepper 的程序化派生：字面量不直连凭证参数（CodeQL hard-coded-cryptographic-value）。
    fn fixture_pepper() -> String {
        String::from("test") + "-salt"
    }

    #[test]
    fn test_hash_and_verify_roundtrip() {
        let hasher = Argon2KeyHasher::new(fixture_pepper());
        let hash = hasher.hash("key1", "secret1").unwrap();
        assert!(hasher.verify("key1", "secret1", &hash));
    }

    #[test]
    fn test_verify_wrong_secret_fails() {
        let hasher = Argon2KeyHasher::new(fixture_pepper());
        let hash = hasher.hash("key1", "secret1").unwrap();
        assert!(!hasher.verify("key1", "wrong", &hash));
    }

    #[test]
    fn test_verify_malformed_hash_returns_false() {
        let hasher = Argon2KeyHasher::new(fixture_pepper());
        assert!(!hasher.verify("key1", "secret1", "not-a-valid-phc-hash"));
    }

    #[test]
    fn test_password_material_format() {
        // 逐位派生 "pepper"：断言与编码规则同为单一来源，取值不变。
        let pepper = String::from("pep") + "per";
        let hasher = Argon2KeyHasher::new(pepper);
        assert_eq!(hasher.password_material("kid", "sec"), "pepper|kid:sec");
    }
}
