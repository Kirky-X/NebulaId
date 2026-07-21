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

use crate::core::types::error::CoreError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum IdFormat {
    #[default]
    Numeric,
    Prefixed,
    Uuid,
}

impl fmt::Display for IdFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdFormat::Numeric => write!(f, "numeric"),
            IdFormat::Prefixed => write!(f, "prefixed"),
            IdFormat::Uuid => write!(f, "uuid"),
        }
    }
}

impl FromStr for IdFormat {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "numeric" => Ok(IdFormat::Numeric),
            "prefixed" => Ok(IdFormat::Prefixed),
            "uuid" => Ok(IdFormat::Uuid),
            _ => Err(CoreError::InvalidIdFormat(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Id(u128);

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 检测是否是 UUID 格式（版本在高位 12-15 位）
        // UUID v7: 版本位 (12-15) 值为 7
        // UUID v4: 版本位 (12-15) 值为 4
        let version_bits = (self.0 >> 76) & 0xF; // 76 = 64 + 12
        if version_bits == 7 || version_bits == 4 {
            // 是 UUID 格式，输出标准字符串格式
            let uuid = Uuid::from_u128(self.0);
            write!(f, "{}", uuid)
        } else {
            // 数值格式（Segment、Snowflake）
            write!(f, "{}", self.0)
        }
    }
}

impl Id {
    pub fn from_u128(value: u128) -> Self {
        Id(value)
    }

    pub fn as_u128(&self) -> u128 {
        self.0
    }

    pub fn from_i64(value: i64) -> Self {
        Id(value as u128)
    }

    pub fn as_i64(&self) -> i64 {
        self.0 as i64
    }

    pub fn from_string(s: &str) -> Result<Self, CoreError> {
        let cleaned = s.trim();
        if cleaned.contains('-') && cleaned.len() == 36 {
            if let Ok(uuid) = Uuid::parse_str(cleaned) {
                return Ok(Id(uuid.as_u128()));
            }
        }

        let value = cleaned
            .parse::<u128>()
            .map_err(|_| CoreError::InvalidIdString(s.to_string()))?;
        Ok(Id(value))
    }

    pub fn to_uuid_v7(&self) -> Uuid {
        Uuid::from_u128(self.0)
    }

    pub fn from_uuid_v7(uuid: Uuid) -> Self {
        Id(uuid.as_u128())
    }

    pub fn from_uuid_v4(uuid: Uuid) -> Self {
        Id(uuid.as_u128())
    }

    pub fn to_prefixed(&self, prefix: &str) -> String {
        format!("{}{}", prefix, self.0)
    }

    pub fn to_hex(&self) -> String {
        format!("{:032x}", self.0)
    }

    pub fn to_base36(&self) -> String {
        const CHARSET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut num = self.0;
        if num == 0 {
            return "0".to_string();
        }
        let mut result = String::new();
        while num > 0 {
            let idx = (num % 36) as usize;
            result.push(CHARSET[idx] as char);
            num /= 36;
        }
        result.chars().rev().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdBatch {
    pub ids: Vec<Id>,
    pub algorithm: AlgorithmType,
    pub biz_tag: String,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

impl IdBatch {
    pub fn new(ids: Vec<Id>, algorithm: AlgorithmType, biz_tag: String) -> Self {
        Self {
            ids,
            algorithm,
            biz_tag,
            generated_at: chrono::Utc::now(),
        }
    }

    pub fn from_u64s(values: &[u64]) -> Self {
        let ids = values.iter().map(|v| Id::from_u128((*v).into())).collect();
        Self {
            ids,
            algorithm: AlgorithmType::Segment,
            biz_tag: String::new(),
            generated_at: chrono::Utc::now(),
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum AlgorithmType {
    #[default]
    Segment,
    Snowflake,
    UuidV7,
    UuidV4,
}

impl fmt::Display for AlgorithmType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AlgorithmType::Segment => write!(f, "segment"),
            AlgorithmType::Snowflake => write!(f, "snowflake"),
            AlgorithmType::UuidV7 => write!(f, "uuid_v7"),
            AlgorithmType::UuidV4 => write!(f, "uuid_v4"),
        }
    }
}

impl FromStr for AlgorithmType {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "segment" => Ok(AlgorithmType::Segment),
            "snowflake" => Ok(AlgorithmType::Snowflake),
            "uuid_v7" | "uuidv7" | "uuid7" => Ok(AlgorithmType::UuidV7),
            "uuid_v4" | "uuidv4" | "uuid4" => Ok(AlgorithmType::UuidV4),
            _ => Err(CoreError::InvalidAlgorithmType(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdMetadata {
    pub timestamp: u64,
    pub datacenter_id: u8,
    pub worker_id: u16,
    pub sequence: u16,
    pub algorithm: AlgorithmType,
    pub biz_tag: String,
}

impl IdMetadata {
    pub fn for_segment(biz_tag: String) -> Self {
        Self {
            timestamp: 0,
            datacenter_id: 0,
            worker_id: 0,
            sequence: 0,
            algorithm: AlgorithmType::Segment,
            biz_tag,
        }
    }

    pub fn for_snowflake(timestamp: u64, datacenter_id: u8, worker_id: u16, sequence: u16) -> Self {
        Self {
            timestamp,
            datacenter_id,
            worker_id,
            sequence,
            algorithm: AlgorithmType::Snowflake,
            biz_tag: String::new(),
        }
    }

    pub fn for_uuid_v7(timestamp: u64) -> Self {
        Self {
            timestamp,
            datacenter_id: 0,
            worker_id: 0,
            sequence: 0,
            algorithm: AlgorithmType::UuidV7,
            biz_tag: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_from_u128() {
        let id = Id::from_u128(12345678901234567890u128);
        assert_eq!(id.as_u128(), 12345678901234567890u128);
    }

    #[test]
    fn test_id_from_string_numeric() {
        let id = Id::from_string("12345678901234567890").unwrap();
        assert_eq!(id.as_u128(), 12345678901234567890u128);
    }

    #[test]
    fn test_id_to_prefixed() {
        let id = Id::from_u128(12345);
        let prefixed = id.to_prefixed("order_");
        assert_eq!(prefixed, "order_12345");
    }

    #[test]
    fn test_id_to_base36() {
        let id = Id::from_u128(36);
        assert_eq!(id.to_base36(), "10");

        let id = Id::from_u128(35);
        assert_eq!(id.to_base36(), "Z");
    }

    #[test]
    fn test_id_format_from_str() {
        assert_eq!(IdFormat::from_str("numeric").unwrap(), IdFormat::Numeric);
        assert_eq!(IdFormat::from_str("prefixed").unwrap(), IdFormat::Prefixed);
        assert_eq!(IdFormat::from_str("uuid").unwrap(), IdFormat::Uuid);
    }

    #[test]
    fn test_algorithm_type_from_str() {
        assert_eq!(
            AlgorithmType::from_str("segment").unwrap(),
            AlgorithmType::Segment
        );
        assert_eq!(
            AlgorithmType::from_str("snowflake").unwrap(),
            AlgorithmType::Snowflake
        );
        assert_eq!(
            AlgorithmType::from_str("uuid_v7").unwrap(),
            AlgorithmType::UuidV7
        );
        assert_eq!(
            AlgorithmType::from_str("uuid_v4").unwrap(),
            AlgorithmType::UuidV4
        );
    }

    // ===== IdFormat Display 全部分支 =====

    #[test]
    fn test_id_format_display_numeric() {
        assert_eq!(IdFormat::Numeric.to_string(), "numeric");
    }

    #[test]
    fn test_id_format_display_prefixed() {
        assert_eq!(IdFormat::Prefixed.to_string(), "prefixed");
    }

    #[test]
    fn test_id_format_display_uuid() {
        assert_eq!(IdFormat::Uuid.to_string(), "uuid");
    }

    #[test]
    fn test_id_format_default_is_numeric() {
        assert_eq!(IdFormat::default(), IdFormat::Numeric);
    }

    // ===== IdFormat FromStr 错误路径 =====

    #[test]
    fn test_id_format_from_str_invalid_returns_error() {
        let result = IdFormat::from_str("binary");
        assert!(result.is_err());
        match result {
            Err(CoreError::InvalidIdFormat(s)) => assert_eq!(s, "binary"),
            other => panic!("expected InvalidIdFormat error, got {other:?}"),
        }
    }

    #[test]
    fn test_id_format_from_str_case_insensitive() {
        assert_eq!(IdFormat::from_str("NUMERIC").unwrap(), IdFormat::Numeric);
        assert_eq!(IdFormat::from_str("Prefixed").unwrap(), IdFormat::Prefixed);
        assert_eq!(IdFormat::from_str("UUID").unwrap(), IdFormat::Uuid);
    }

    // ===== Id::from_i64 / as_i64 =====

    #[test]
    fn test_id_from_i64_positive() {
        let id = Id::from_i64(12345);
        assert_eq!(id.as_i64(), 12345);
        assert_eq!(id.as_u128(), 12345u128);
    }

    #[test]
    fn test_id_from_i64_zero() {
        let id = Id::from_i64(0);
        assert_eq!(id.as_i64(), 0);
        assert_eq!(id.as_u128(), 0);
    }

    #[test]
    fn test_id_from_i64_negative_truncates_via_cast() {
        // i64 负数 cast 为 u128 会变成巨大正数（按 production 行为）
        let id = Id::from_i64(-1);
        assert_eq!(id.as_u128(), u128::MAX);
    }

    // ===== Id::from_string（UUID 路径 + 错误路径） =====

    #[test]
    fn test_id_from_string_valid_uuid() {
        let uuid_str = "018e6c5c-8e0f-7c4d-9a3b-1234567890ab";
        let id = Id::from_string(uuid_str).unwrap();
        let parsed_uuid = Uuid::parse_str(uuid_str).unwrap();
        assert_eq!(id.as_u128(), parsed_uuid.as_u128());
    }

    #[test]
    fn test_id_from_string_uuid_with_whitespace_is_trimmed() {
        let uuid_str = "  018e6c5c-8e0f-7c4d-9a3b-1234567890ab  ";
        let id = Id::from_string(uuid_str).unwrap();
        let clean = uuid_str.trim();
        let parsed_uuid = Uuid::parse_str(clean).unwrap();
        assert_eq!(id.as_u128(), parsed_uuid.as_u128());
    }

    #[test]
    fn test_id_from_string_invalid_uuid_falls_back_to_numeric() {
        // 长度为 36 且含 '-' 但不是合法 UUID → 应回退到 u128 解析
        let s = "------------------------------1234"; // 36 chars, 含 '-'，但不是 UUID
                                                      // 这个字符串不能解析为 u128，应返回 InvalidIdString
        let result = Id::from_string(s);
        assert!(result.is_err());
        match result {
            Err(CoreError::InvalidIdString(_)) => {}
            other => panic!("expected InvalidIdString, got {other:?}"),
        }
    }

    #[test]
    fn test_id_from_string_invalid_numeric_returns_error() {
        let result = Id::from_string("not-a-number");
        assert!(result.is_err());
        match result {
            Err(CoreError::InvalidIdString(s)) => assert_eq!(s, "not-a-number"),
            other => panic!("expected InvalidIdString, got {other:?}"),
        }
    }

    #[test]
    fn test_id_from_string_empty_returns_error() {
        let result = Id::from_string("");
        assert!(result.is_err());
    }

    #[test]
    fn test_id_from_string_too_large_u128_returns_error() {
        // u128 + 1 → 溢出
        let result = Id::from_string("340282366920938463463374607431768211456"); // 2^128
        assert!(result.is_err());
    }

    // ===== Id::from_uuid_v4 / to_uuid_v7 roundtrip =====

    #[test]
    fn test_id_from_uuid_v4_roundtrip() {
        let uuid = Uuid::new_v4();
        let id = Id::from_uuid_v4(uuid);
        let back = id.to_uuid_v7();
        assert_eq!(back, uuid);
        assert_eq!(back.get_version(), Some(uuid::Version::Random));
    }

    #[test]
    fn test_id_from_uuid_v7_roundtrip() {
        let uuid = Uuid::now_v7();
        let id = Id::from_uuid_v7(uuid);
        let back = id.to_uuid_v7();
        assert_eq!(back, uuid);
        assert_eq!(back.get_version(), Some(uuid::Version::SortRand));
    }

    // ===== Id::to_hex =====

    #[test]
    fn test_id_to_hex_zero() {
        let id = Id::from_u128(0);
        assert_eq!(id.to_hex(), "00000000000000000000000000000000");
    }

    #[test]
    fn test_id_to_hex_max_u128() {
        let id = Id::from_u128(u128::MAX);
        assert_eq!(id.to_hex(), "ffffffffffffffffffffffffffffffff");
    }

    #[test]
    fn test_id_to_hex_value() {
        let id = Id::from_u128(0x1234abcd);
        assert_eq!(id.to_hex(), "0000000000000000000000001234abcd");
    }

    #[test]
    fn test_id_to_hex_is_32_chars() {
        let id = Id::from_u128(42);
        let hex = id.to_hex();
        assert_eq!(hex.len(), 32);
    }

    // ===== Id::to_base36 zero 路径 =====

    #[test]
    fn test_id_to_base36_zero() {
        let id = Id::from_u128(0);
        assert_eq!(id.to_base36(), "0");
    }

    #[test]
    fn test_id_to_base36_max_u128() {
        let id = Id::from_u128(u128::MAX);
        let result = id.to_base36();
        // 验证可逆：用 base36 解析回来应等于 u128::MAX
        let parsed = u128::from_str_radix(&result, 36).unwrap();
        assert_eq!(parsed, u128::MAX);
    }

    #[test]
    fn test_id_to_base36_single_digit() {
        // 0-9 应保持单字符
        for n in 0u128..10 {
            let id = Id::from_u128(n);
            let s = id.to_base36();
            assert_eq!(s.len(), 1);
            assert_eq!(
                s.chars().next().unwrap(),
                char::from_digit(n as u32, 36).unwrap()
            );
        }
    }

    #[test]
    fn test_id_to_base36_letters_a_to_z() {
        // 10-35 应映射到 A-Z
        for n in 10u128..36 {
            let id = Id::from_u128(n);
            let s = id.to_base36();
            assert_eq!(s.len(), 1);
            let expected = char::from_digit(n as u32, 36).unwrap().to_ascii_uppercase();
            assert_eq!(s.chars().next().unwrap(), expected);
        }
    }

    // ===== Id::Display（UUID 格式 vs 数值格式） =====

    #[test]
    fn test_id_display_numeric_for_small_value() {
        let id = Id::from_u128(12345);
        // 12345 的高位版本位 (76-79) 不是 4 或 7，应输出数值
        assert_eq!(id.to_string(), "12345");
    }

    #[test]
    fn test_id_display_uuid_format_for_v7() {
        let uuid = Uuid::now_v7();
        let id = Id::from_uuid_v7(uuid);
        // UUID v7 应输出标准 UUID 字符串
        assert_eq!(id.to_string(), uuid.to_string());
        assert_eq!(id.to_string().len(), 36);
    }

    #[test]
    fn test_id_display_uuid_format_for_v4() {
        let uuid = Uuid::new_v4();
        let id = Id::from_uuid_v4(uuid);
        // UUID v4 应输出标准 UUID 字符串
        assert_eq!(id.to_string(), uuid.to_string());
        assert_eq!(id.to_string().len(), 36);
    }

    // ===== IdBatch::from_u64s / is_empty / len =====

    #[test]
    fn test_id_batch_from_u64s() {
        let values = vec![1u64, 2, 3, 100, 1000];
        let batch = IdBatch::from_u64s(&values);
        assert_eq!(batch.len(), 5);
        assert!(!batch.is_empty());
        assert_eq!(batch.algorithm, AlgorithmType::Segment);
        assert_eq!(batch.biz_tag, "");
        // 验证每个 ID 的值
        for (i, v) in values.iter().enumerate() {
            assert_eq!(batch.ids[i].as_u128(), *v as u128);
        }
    }

    #[test]
    fn test_id_batch_from_u64s_empty() {
        let batch = IdBatch::from_u64s(&[]);
        assert_eq!(batch.len(), 0);
        assert!(batch.is_empty());
        assert_eq!(batch.algorithm, AlgorithmType::Segment);
    }

    #[test]
    fn test_id_batch_new_with_algorithm_and_biz_tag() {
        let ids = vec![Id::from_u128(1), Id::from_u128(2)];
        let batch = IdBatch::new(ids, AlgorithmType::Snowflake, "order".to_string());
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.algorithm, AlgorithmType::Snowflake);
        assert_eq!(batch.biz_tag, "order");
        // generated_at 应为当前时间附近
        let now = chrono::Utc::now();
        let diff = now.signed_duration_since(batch.generated_at);
        assert!(
            diff.num_seconds().abs() < 5,
            "generated_at should be recent"
        );
    }

    // ===== AlgorithmType Display 全部分支 =====

    #[test]
    fn test_algorithm_type_display_segment() {
        assert_eq!(AlgorithmType::Segment.to_string(), "segment");
    }

    #[test]
    fn test_algorithm_type_display_snowflake() {
        assert_eq!(AlgorithmType::Snowflake.to_string(), "snowflake");
    }

    #[test]
    fn test_algorithm_type_display_uuid_v7() {
        assert_eq!(AlgorithmType::UuidV7.to_string(), "uuid_v7");
    }

    #[test]
    fn test_algorithm_type_display_uuid_v4() {
        assert_eq!(AlgorithmType::UuidV4.to_string(), "uuid_v4");
    }

    #[test]
    fn test_algorithm_type_default_is_segment() {
        assert_eq!(AlgorithmType::default(), AlgorithmType::Segment);
    }

    // ===== AlgorithmType FromStr 全部分支（别名 + 错误） =====

    #[test]
    fn test_algorithm_type_from_str_uuid_v7_aliases() {
        assert_eq!(
            AlgorithmType::from_str("uuidv7").unwrap(),
            AlgorithmType::UuidV7
        );
        assert_eq!(
            AlgorithmType::from_str("uuid7").unwrap(),
            AlgorithmType::UuidV7
        );
    }

    #[test]
    fn test_algorithm_type_from_str_uuid_v4_aliases() {
        assert_eq!(
            AlgorithmType::from_str("uuidv4").unwrap(),
            AlgorithmType::UuidV4
        );
        assert_eq!(
            AlgorithmType::from_str("uuid4").unwrap(),
            AlgorithmType::UuidV4
        );
    }

    #[test]
    fn test_algorithm_type_from_str_case_insensitive() {
        assert_eq!(
            AlgorithmType::from_str("SEGMENT").unwrap(),
            AlgorithmType::Segment
        );
        assert_eq!(
            AlgorithmType::from_str("Snowflake").unwrap(),
            AlgorithmType::Snowflake
        );
        assert_eq!(
            AlgorithmType::from_str("UUID_V7").unwrap(),
            AlgorithmType::UuidV7
        );
    }

    #[test]
    fn test_algorithm_type_from_str_invalid_returns_error() {
        let result = AlgorithmType::from_str("redis");
        assert!(result.is_err());
        match result {
            Err(CoreError::InvalidAlgorithmType(s)) => assert_eq!(s, "redis"),
            other => panic!("expected InvalidAlgorithmType, got {other:?}"),
        }
    }

    // ===== IdMetadata =====

    #[test]
    fn test_id_metadata_for_segment() {
        let metadata = IdMetadata::for_segment("order".to_string());
        assert_eq!(metadata.algorithm, AlgorithmType::Segment);
        assert_eq!(metadata.biz_tag, "order");
        assert_eq!(metadata.timestamp, 0);
        assert_eq!(metadata.datacenter_id, 0);
        assert_eq!(metadata.worker_id, 0);
        assert_eq!(metadata.sequence, 0);
    }

    #[test]
    fn test_id_metadata_for_snowflake() {
        let metadata = IdMetadata::for_snowflake(1234567890, 5, 10, 42);
        assert_eq!(metadata.algorithm, AlgorithmType::Snowflake);
        assert_eq!(metadata.timestamp, 1234567890);
        assert_eq!(metadata.datacenter_id, 5);
        assert_eq!(metadata.worker_id, 10);
        assert_eq!(metadata.sequence, 42);
        assert_eq!(metadata.biz_tag, "");
    }

    #[test]
    fn test_id_metadata_for_uuid_v7() {
        let metadata = IdMetadata::for_uuid_v7(1700000000);
        assert_eq!(metadata.algorithm, AlgorithmType::UuidV7);
        assert_eq!(metadata.timestamp, 1700000000);
        assert_eq!(metadata.datacenter_id, 0);
        assert_eq!(metadata.worker_id, 0);
        assert_eq!(metadata.sequence, 0);
        assert_eq!(metadata.biz_tag, "");
    }

    // ===== Id::to_prefixed 边界 =====

    #[test]
    fn test_id_to_prefixed_empty_prefix() {
        let id = Id::from_u128(42);
        assert_eq!(id.to_prefixed(""), "42");
    }

    #[test]
    fn test_id_to_prefixed_zero_value() {
        let id = Id::from_u128(0);
        assert_eq!(id.to_prefixed("id_"), "id_0");
    }

    #[test]
    fn test_id_to_prefixed_max_u128() {
        let id = Id::from_u128(u128::MAX);
        assert_eq!(id.to_prefixed("max_"), format!("max_{}", u128::MAX));
    }

    // ===== Id as_u128 / from_u128 边界 =====

    #[test]
    fn test_id_as_u128_max() {
        let id = Id::from_u128(u128::MAX);
        assert_eq!(id.as_u128(), u128::MAX);
    }

    #[test]
    fn test_id_as_u128_zero() {
        let id = Id::from_u128(0);
        assert_eq!(id.as_u128(), 0);
    }

    // ===== IdBatch len() / is_empty() 互斥 =====

    #[test]
    fn test_id_batch_len_zero_iff_empty() {
        let empty = IdBatch::new(vec![], AlgorithmType::Segment, String::new());
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());

        let non_empty = IdBatch::new(
            vec![Id::from_u128(1)],
            AlgorithmType::Segment,
            String::new(),
        );
        assert_eq!(non_empty.len(), 1);
        assert!(!non_empty.is_empty());
    }
}
