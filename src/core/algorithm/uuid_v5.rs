// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! RFC 9562 §5.5 name-based UUID v5 确定性派生。
//!
//! 与 [`super::uuid_v8`] 的发号语义并列而分岔：v8 每次生成必然不同
//! （时间戳 + 计数器 + 随机熵），v5 同输入恒同输出（SHA-1(namespace ||
//! name) 取高 128 位，version 置 0b0101、variant 置 0b10）——服务于
//! 「按业务身份派生 ID」的调用方（如 etyma 词典实体按 (lemma, language)
//! 派生；其 340,566 个存量 ID 的数值即标准 v5 输出，跨仓兼容锚点见
//! 测试 [`test_etyma_ecdict_anchor_abandon`]）。
//!
//! 纯函数：无状态、无时钟、无 IO、无锁。feature `uuid-v5` 门控。
//!
//! # 与算法族的关系
//!
//! 本模块刻意**不**实现 [`crate::core::algorithm::traits::IdAlgorithm`]、
//! 不进 `AlgorithmRouter`：派生失败没有「换一个发号算法」的可动作语义
//! （router fallback 链会产出数值错误的非 v5 ID），且 `AlgorithmType` 是
//! 发号算法枚举（绑定 fallback / degradation / set_algorithm 白名单）。
//! UUID v4/v7 在本仓同样不是算法族成员（仅 `FromStr` 输入别名，见
//! `core/types/id.rs`）。

use uuid::Uuid;

use crate::core::types::id::Id;
use crate::core::types::Result;

/// RFC 9562 §5.5 确定性派生：v5 = SHA-1(namespace 网络序 16 字节 || name)
/// 取高 128 位，version 置 0b0101、variant 置 0b10。
///
/// 同 (namespace, name) 恒同输出。name 以 UTF-8 字节参与——RFC 9562 的
/// 「canonical sequence of octets」对文本名即 UTF-8 编码，与 etyma 等
/// `Uuid::new_v5` 消费方同口径（同 crate 同版本，数值逐位兼容）。
///
/// 错误面：当前无失败路径（`Uuid::new_v5` 不返回 Result）。保留 `Result`
/// 返回类型以稳定签名（未来入参校验——如空 name 拒绝——不必惊动调用方），
/// 不为假想的失败提前造错误变体（规则 6）。
pub fn derive_uuid_v5(namespace: Uuid, name: &str) -> Result<Id> {
    Ok(Id::from_uuid_v8(Uuid::new_v5(&namespace, name.as_bytes())))
}

/// 批量确定性派生：多组 (namespace, name) 各派生一次，按序返回，长度与
/// 输入一致。
///
/// 与发号 batch 的「size 个互异新 ID」是不同语义，刻意以不同函数名区分，
/// 防止误用；调用方按导入批次切分输入即可。
pub fn derive_uuid_v5_batch(pairs: &[(Uuid, &str)]) -> Result<Vec<Id>> {
    pairs
        .iter()
        .map(|(namespace, name)| derive_uuid_v5(*namespace, name))
        .collect()
}

/// 字节级确定性派生：与 [`derive_uuid_v5`] 同算法（SHA-1(namespace 网络序
/// || name)），差异仅在 name 已是原始字节、不再经 UTF-8 编解码。
///
/// 消费方把 v5 键建在非文本负载上时（如 u64 偏移的 16 字节大端缓冲，含
/// ≥0x80 字节、非法 UTF-8）必须走本函数；与 `Uuid::new_v5(ns, bytes)` 逐位
/// 一致，数值契约不因切换原语而变。
pub fn derive_uuid_v5_bytes(namespace: Uuid, name: &[u8]) -> Result<Id> {
    Ok(Id::from_uuid_v8(Uuid::new_v5(&namespace, name)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::id::IdFormat;
    use uuid::Uuid;

    /// etyma ECDICT 词条命名空间（etyma-domain/src/word_id.rs:17，独立第三方常量）。
    fn ecdict_namespace() -> Uuid {
        Uuid::from_bytes([
            0x47, 0x9e, 0x67, 0xb5, 0x8d, 0x48, 0x51, 0x9e, 0xb7, 0x5c, 0x3c, 0x17, 0x65, 0xc4,
            0x5e, 0x9b,
        ])
    }

    fn canonical(id: &crate::core::types::id::Id) -> String {
        id.render(&IdFormat::Uuid, None, None)
            .expect("uuid format render must succeed")
    }

    /// 锚点 1（etyma 仓内钉死值，340,566 存量 ID 依赖）：name 格式
    /// `ecdict|{lemma}|en` 是 etyma 的领域规则，本模块只提供标准算法。
    #[test]
    fn test_etyma_ecdict_anchor_abandon() {
        let id = derive_uuid_v5(ecdict_namespace(), "ecdict|abandon|en").unwrap();
        assert_eq!(canonical(&id), "21e7d32f-5500-5e4a-a943-219059f73c94");
    }

    /// 锚点 2（Python hashlib SHA-1 独立基准，与本实现零共同代码）。
    #[test]
    fn test_python_hashlib_anchor_ad() {
        let id = derive_uuid_v5(ecdict_namespace(), "ecdict|ad|en").unwrap();
        assert_eq!(canonical(&id), "a696b85d-56db-54a7-ada8-282a93ff27f4");
    }

    /// 位布局形貌：version nibble = 5、variant = 0b10（RFC 9562 §5.5）。
    #[test]
    fn test_version_and_variant_bits() {
        let id = derive_uuid_v5(ecdict_namespace(), "ecdict|ad|en").unwrap();
        let v = id.to_uuid_v8().as_u128();
        assert_eq!((v >> 76) & 0xF, 5, "version nibble must be 5");
        assert_eq!((v >> 62) & 0x3, 0b10, "variant must be 0b10");
    }

    /// 同输入恒同输出（确定性是唯一核心属性，重复调用不得漂移）。
    #[test]
    fn test_same_input_same_output() {
        let ns = ecdict_namespace();
        let a = derive_uuid_v5(ns, "ecdict|ad|en").unwrap();
        let b = derive_uuid_v5(ns, "ecdict|ad|en").unwrap();
        assert_eq!(a, b);
    }

    /// 批量与逐条调用逐位一致（按序返回，长度与输入一致）。
    #[test]
    fn test_batch_matches_sequential() {
        let ns = ecdict_namespace();
        let pairs = [
            (ns, "ecdict|abandon|en"),
            (ns, "ecdict|ad|en"),
            (ns, "ecdict|café|en"),
        ];
        let batch = derive_uuid_v5_batch(&pairs).unwrap();
        assert_eq!(batch.len(), pairs.len());
        for (id, (ns, name)) in batch.iter().zip(pairs.iter()) {
            assert_eq!(*id, derive_uuid_v5(*ns, name).unwrap());
        }
    }

    /// 边界：非 ASCII name 与 nil namespace 均正常产出（Python hashlib
    /// 独立基准），不 panic、值确定。
    #[test]
    fn test_non_ascii_name_and_nil_namespace() {
        let id = derive_uuid_v5(ecdict_namespace(), "ecdict|café|en").unwrap();
        assert_eq!(canonical(&id), "1f2b8aeb-88cf-53b5-a9e6-bf8c0ba42f89");

        let nil_id = derive_uuid_v5(Uuid::nil(), "etyma.dev/sense/v1").unwrap();
        assert_eq!(canonical(&nil_id), "ec7527a2-140f-51b0-9d78-23badb6adcf8");
    }

    /// 字节级入口与文本入口口径一致：ASCII name 走两条路径同值，且非 UTF-8
    /// 负载（u64 大端缓冲形）与 `Uuid::new_v5(ns, bytes)` 逐位一致。
    #[test]
    fn test_bytes_entry_matches_uuid_crate() {
        let ns = ecdict_namespace();

        // ASCII 负载应能经文本入口复算（两条路径同算法）。
        let via_str = derive_uuid_v5(ns, "ecdict|ad|en").unwrap();
        let via_bytes = derive_uuid_v5_bytes(ns, b"ecdict|ad|en").unwrap();
        assert_eq!(via_str, via_bytes);

        // u64 大端 + 8×NUL 缓冲（etyma WordNet synset_uuid 的构造，含 ≥0x80
        // 字节、非法 UTF-8）：与 uuid crate 逐位一致是迁移的数值契约。
        let mut buf = [0u8; 16];
        buf[..8].copy_from_slice(&12345678u64.to_be_bytes());
        let id = derive_uuid_v5_bytes(ns, &buf).unwrap();
        assert_eq!(
            id.to_uuid_v8(),
            Uuid::new_v5(&ns, &buf),
            "byte-level derive must be bit-identical to Uuid::new_v5"
        );

        // name 为空（纯 namespace 哈希）不为错误路径。
        let empty = derive_uuid_v5_bytes(ns, &[]).unwrap();
        assert_eq!(empty.to_uuid_v8(), Uuid::new_v5(&ns, &[]));
    }
}
