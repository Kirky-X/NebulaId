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

use crate::types::error::CoreError;
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
}
