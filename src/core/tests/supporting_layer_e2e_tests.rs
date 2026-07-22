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

//! 核心支撑层端到端测试
//!
//! 覆盖 `temp/功能场景穷举分析.md` 第 2 节中描述的跨模块协同场景：
//! - Config 从 TOML 文件加载 + 环境变量展开 + validate 完整链路
//! - Config 所有 validate 失败路径（端口/dc_id/连接数/限流/算法位数/batch 上限）
//! - Config::merge 优先级语义（非默认值字段覆盖）
//! - Id 类型跨格式端到端转换（u128 ↔ UUID 字符串 ↔ prefixed/hex/base36）
//! - AlgorithmType 别名解析（uuidv7/uuid7/uuidv4/uuid4 + 大小写不敏感）
//! - CoreError i18n 跨 locale 翻译链路
//!
//! 这些测试聚焦跨子模块协同，避免与 app_config.rs / id.rs / error.rs 内
//! `#[cfg(test)] mod tests` 的单元测试重复（那些覆盖单函数边界）。

use crate::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use crate::core::config::Config;
use crate::core::config::{AlgorithmConfig, ConfigError};
use crate::core::types::{AlgorithmType, CoreError, Id, IdBatch, IdFormat};
use std::str::FromStr;
use std::sync::Mutex;
use tempfile::tempdir;
use uuid::Uuid;

// =============================================================================
// Config 端到端
// =============================================================================

/// E2E-CFG-001: Config::load_from_file 完整路径——TOML 解析 + 环境变量展开 + validate。
///
/// 覆盖功能场景穷举分析第 2.2 节配置文件加载行：
/// - 读取 TOML 文件
/// - ${VAR_NAME} 语法环境变量替换
/// - 通过 validate 校验
///
/// 与 app_config.rs 单元测试的区别：单元测试分步验证 expand_env_vars / parse / validate，
/// 这里验证完整链路：写入临时 TOML 文件 → load_from_file → 字段值正确。
///
/// 实现策略：用 `Config::default()` + 手动修改特定字段，再 `toml::to_string` 生成完整
/// TOML。这样能保证所有必填字段都齐全，避免手写 TOML 遗漏字段导致的解析错误。
/// 修改字段值用于验证 load_from_file 后能正确读取（区别于 default）。
#[tokio::test]
async fn e2e_config_load_from_file_with_env_expansion_and_validation() {
    let dir = tempdir().expect("E2E: tempdir should succeed");
    let file_path = dir.path().join("test_config.toml");

    // 构造与 default 不同的配置，用于验证 load_from_file 真正读取字段而非依赖默认值
    let mut original = Config::default();
    original.app.http_port = 18080;
    original.app.grpc_port = 19091;
    original.app.dc_id = 5;
    original.app.worker_id = 10;
    original.algorithm.default = "snowflake".to_string();
    original.algorithm.snowflake.clock_drift_threshold_ms = 2000;
    original.rate_limit.default_rps = 100;
    original.rate_limit.burst_size = 200;
    original.batch_generate.max_batch_size = 5000;
    // 设置一个固定的 url 值，便于后续替换为 ${VAR} 占位符
    original.database.url = "postgres://user:placeholder@localhost:5432/nebulaid".to_string();

    let toml_content = toml::to_string(&original).expect("E2E: serialize Config to TOML");

    // 设置环境变量供 ${VAR} 展开（串行化避免与其他测试的环境变量操作竞争）
    let _env_guard = E2E_LOCALE_LOCK.lock().unwrap();
    std::env::set_var("E2E_TEST_DB_PASSWORD", "supersecret");
    let toml_with_var = toml_content.replace(
        "postgres://user:placeholder@",
        "postgres://user:${E2E_TEST_DB_PASSWORD}@",
    );

    std::fs::write(&file_path, &toml_with_var).expect("E2E: write TOML file should succeed");

    let config = Config::load_from_file(file_path.to_str().unwrap())
        .expect("E2E: load_from_file should succeed");

    // 验证字段
    assert_eq!(config.app.http_port, 18080);
    assert_eq!(config.app.grpc_port, 19091);
    assert_eq!(config.app.dc_id, 5);
    assert_eq!(config.app.worker_id, 10);
    assert_eq!(config.algorithm.default, "snowflake");
    assert_eq!(config.algorithm.snowflake.clock_drift_threshold_ms, 2000);
    assert_eq!(config.rate_limit.default_rps, 100);
    assert_eq!(config.batch_generate.max_batch_size, 5000);

    // 验证环境变量已展开（断言消息不输出完整 URL，避免凭证泄漏到 CI 日志）
    assert!(
        config.database.url.contains("supersecret"),
        "E2E: env var should be expanded in database.url"
    );
    assert!(
        !config.database.url.contains("${E2E_TEST_DB_PASSWORD}"),
        "E2E: env var placeholder should be fully replaced"
    );

    std::env::remove_var("E2E_TEST_DB_PASSWORD");
}

/// E2E-CFG-002: Config::load_from_file 在文件不存在时返回 FileError。
///
/// 覆盖功能场景穷举分析第 2.2 节"文件不存在 → FileError"。
#[tokio::test]
async fn e2e_config_load_from_file_missing_returns_file_error() {
    let result = Config::load_from_file("/nonexistent/path/to/config.toml");
    match result {
        Err(ConfigError::FileError(msg)) => {
            assert!(
                msg.contains("No such file") || msg.contains("Not found") || !msg.is_empty(),
                "E2E: FileError should describe IO failure, got: {}",
                msg
            );
        }
        other => panic!("E2E: expected FileError, got {:?}", other),
    }
}

/// E2E-CFG-003: Config::load_from_file 在 TOML 格式错误时返回 InvalidValue。
#[tokio::test]
async fn e2e_config_load_from_file_invalid_toml_returns_invalid_value() {
    let dir = tempdir().expect("E2E: tempdir");
    let file_path = dir.path().join("invalid.toml");
    std::fs::write(&file_path, "not = valid = toml = syntax =").unwrap();

    let result = Config::load_from_file(file_path.to_str().unwrap());
    assert!(
        matches!(result, Err(ConfigError::InvalidValue(_))),
        "E2E: invalid TOML should return InvalidValue, got {:?}",
        result
    );
}

/// E2E-CFG-004: Config::validate 在 http_port=0 时返回 InvalidValue。
///
/// 覆盖功能场景穷举分析第 2.2 节配置校验行的"端口 1-65535"边界。
#[test]
fn e2e_config_validate_http_port_zero_fails() {
    let mut config = Config::default();
    config.app.http_port = 0;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// E2E-CFG-005: Config::validate 在 dc_id > 31 时返回 InvalidValue。
#[test]
fn e2e_config_validate_dc_id_over_31_fails() {
    let mut config = Config::default();
    config.app.dc_id = 32;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// E2E-CFG-006: Config::validate 在 dc_id == 31 时通过（边界）。
#[test]
fn e2e_config_validate_dc_id_31_passes() {
    let mut config = Config::default();
    config.app.dc_id = 31;
    assert!(config.validate().is_ok());
}

/// E2E-CFG-007: Config::validate 在 min_connections > max_connections 时失败。
#[test]
fn e2e_config_validate_min_gt_max_connections_fails() {
    let mut config = Config::default();
    config.database.min_connections = 200;
    config.database.max_connections = 100;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// E2E-CFG-008: Config::validate 在 rate_limit 启用但 burst_size > 10×rps 时失败。
#[test]
fn e2e_config_validate_burst_exceeds_10x_rps_fails() {
    let mut config = Config::default();
    config.rate_limit.enabled = true;
    config.rate_limit.default_rps = 10;
    config.rate_limit.burst_size = 200; // 200 > 10 × 10
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// E2E-CFG-009: Config::validate 在 snowflake 总位数 ≥ 64 时失败。
#[test]
fn e2e_config_validate_snowflake_total_bits_over_64_fails() {
    let mut config = Config::default();
    config.algorithm.snowflake.datacenter_id_bits = 20;
    config.algorithm.snowflake.worker_id_bits = 24;
    config.algorithm.snowflake.sequence_bits = 20; // total = 64
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// E2E-CFG-010: Config::validate 在 batch_generate.max_batch_size > 10000 时失败。
#[test]
fn e2e_config_validate_batch_size_over_10000_fails() {
    let mut config = Config::default();
    config.batch_generate.max_batch_size = 10001;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// E2E-CFG-011: Config::validate 在 batch_generate.max_batch_size == 10000 时通过（边界）。
#[test]
fn e2e_config_validate_batch_size_10000_passes() {
    let mut config = Config::default();
    config.batch_generate.max_batch_size = 10000;
    assert!(config.validate().is_ok());
}

/// E2E-CFG-012: Config::validate 在 algorithm.default 不是合法值时失败。
#[test]
fn e2e_config_validate_invalid_default_algorithm_fails() {
    let mut config = Config::default();
    config.algorithm.default = "redis".to_string();
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// E2E-CFG-013: Config::validate 在 switch_threshold 越界时失败。
#[test]
fn e2e_config_validate_switch_threshold_out_of_range_fails() {
    let mut config = Config::default();
    config.algorithm.segment.switch_threshold = 1.5;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// E2E-CFG-014: Config::merge 优先级——非默认值字段覆盖。
///
/// 覆盖功能场景穷举分析第 2.2 节配置合并行的"非默认值字段覆盖"。
/// Config::merge 用默认值作为哨兵判断"用户是否显式设置"，所以显式设为
/// 默认值的字段会被忽略（已知限制，本测试钉住此行为）。
#[test]
fn e2e_config_merge_overrides_non_default_fields() {
    let mut base = Config::default();
    base.app.http_port = 8080;

    let mut other = Config::default();
    other.app.http_port = 9090;
    other.app.dc_id = 7;

    base.merge(other);

    assert_eq!(
        base.app.http_port, 9090,
        "E2E: non-default http_port should override"
    );
    assert_eq!(base.app.dc_id, 7, "E2E: non-default dc_id should override");
}

/// E2E-CFG-015: Config::merge 在 other 字段等于默认值时不覆盖。
///
/// 钉住 merge 的"默认值哨兵"语义：other 中显式设为默认值的字段不会覆盖 base。
#[test]
fn e2e_config_merge_default_value_does_not_override() {
    let mut base = Config::default();
    base.app.http_port = 7777; // 非默认值

    let other = Config::default(); // http_port == 8080（默认值）

    base.merge(other);

    assert_eq!(
        base.app.http_port, 7777,
        "E2E: default-value field in `other` should not override base's explicit value"
    );
}

// =============================================================================
// Id 类型跨格式端到端
// =============================================================================

/// E2E-ID-001: 从真实 Snowflake 算法生成的 ID 转换为多种格式并反向解析。
///
/// 覆盖功能场景穷举分析第 2.7 节 ID 类型行的"多格式解析与展示"。
/// 这条路径验证：算法生成 → Id 包装 → to_string / to_hex / to_base36 / to_prefixed
/// 全部格式化接口在真实算法输出上工作正常。
#[tokio::test]
async fn e2e_id_format_conversion_roundtrip_from_real_algorithm() {
    let config = Config::default();
    let algorithm: Box<dyn IdAlgorithm> = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await
        .expect("E2E: build Snowflake");

    let ctx = GenerateContext {
        workspace_id: "ws-e2e".to_string(),
        group_id: "g-e2e".to_string(),
        biz_tag: "bt-e2e".to_string(),
        format: IdFormat::Numeric,
        prefix: None,
    };

    let id = algorithm
        .generate(&ctx)
        .await
        .expect("E2E: generate should succeed");
    let value = id.as_u128();
    assert!(value > 0);

    // 数值字符串 roundtrip
    let numeric_str = id.to_string();
    let parsed = Id::from_string(&numeric_str).expect("E2E: parse numeric string");
    assert_eq!(parsed.as_u128(), value);

    // hex roundtrip
    let hex = id.to_hex();
    assert_eq!(hex.len(), 32, "E2E: hex should be 32 chars");
    let hex_value = u128::from_str_radix(&hex, 16).unwrap();
    assert_eq!(hex_value, value);

    // base36 roundtrip
    let b36 = id.to_base36();
    let b36_value = u128::from_str_radix(&b36, 36).unwrap();
    assert_eq!(b36_value, value);

    // prefixed（不可逆，但可解析前缀+数值）
    let prefixed = id.to_prefixed("order_");
    assert!(prefixed.starts_with("order_"));
    let stripped = prefixed.strip_prefix("order_").unwrap();
    let prefixed_value: u128 = stripped.parse().unwrap();
    assert_eq!(prefixed_value, value);
}

/// E2E-ID-002: UUID v7 字符串 roundtrip 通过 Id::from_string 解析。
///
/// 覆盖功能场景穷举分析第 2.7 节 ID 类型行的"36 位含 - 按 UUID 解析"。
#[test]
fn e2e_id_from_string_uuid_v7_roundtrip() {
    let uuid = Uuid::now_v7();
    let uuid_str = uuid.to_string();
    assert_eq!(uuid_str.len(), 36);

    let id = Id::from_string(&uuid_str).expect("E2E: parse UUID v7 string");
    assert_eq!(id.as_u128(), uuid.as_u128());

    // Id::to_string 应输出 UUID 格式（版本位 7）
    let back = id.to_string();
    assert_eq!(back, uuid_str);
}

/// E2E-ID-003: UUID v4 字符串通过 Id::from_string 解析后保持 v4 版本位。
#[test]
fn e2e_id_from_string_uuid_v4_preserves_version() {
    let uuid = Uuid::new_v4();
    let id = Id::from_string(&uuid.to_string()).expect("E2E: parse UUID v4 string");
    let back = id.to_uuid_v7();
    assert_eq!(back.get_version(), Some(uuid::Version::Random));
}

/// E2E-ID-004: Id::from_string 在空字符串 / 非数字 / 非UUID 时返回 InvalidIdString。
///
/// 覆盖功能场景穷举分析第 2.7 节 ID 类型行的"from_string 解析失败 → InvalidIdString"。
#[test]
fn e2e_id_from_string_invalid_returns_error() {
    assert!(matches!(
        Id::from_string(""),
        Err(CoreError::InvalidIdString(_))
    ));
    assert!(matches!(
        Id::from_string("not-a-uuid-not-a-number"),
        Err(CoreError::InvalidIdString(_))
    ));
}

/// E2E-ID-005: IdBatch::from_u64s 与 IdBatch::new 互转。
#[test]
fn e2e_id_batch_from_u64s_and_new_consistent() {
    let values: Vec<u64> = vec![1, 100, 1000, u64::MAX];
    let batch = IdBatch::from_u64s(&values);
    assert_eq!(batch.len(), 4);
    assert_eq!(batch.algorithm, AlgorithmType::Segment);
    for (i, v) in values.iter().enumerate() {
        assert_eq!(batch.ids[i].as_u128(), *v as u128);
    }
}

// =============================================================================
// AlgorithmType 别名解析端到端
// =============================================================================

/// E2E-ALG-001: AlgorithmType::from_str 接受所有别名与大小写变体。
///
/// 覆盖功能场景穷举分析第 2.7 节"算法类型解析"行：
/// - 大小写不敏感
/// - 支持 uuidv7 / uuid7 别名
#[test]
fn e2e_algorithm_type_from_str_all_aliases_and_case_variants() {
    // 标准名称
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

    // 别名
    assert_eq!(
        AlgorithmType::from_str("uuidv7").unwrap(),
        AlgorithmType::UuidV7
    );
    assert_eq!(
        AlgorithmType::from_str("uuid7").unwrap(),
        AlgorithmType::UuidV7
    );
    assert_eq!(
        AlgorithmType::from_str("uuidv4").unwrap(),
        AlgorithmType::UuidV4
    );
    assert_eq!(
        AlgorithmType::from_str("uuid4").unwrap(),
        AlgorithmType::UuidV4
    );

    // 大小写不敏感
    assert_eq!(
        AlgorithmType::from_str("SEGMENT").unwrap(),
        AlgorithmType::Segment
    );
    assert_eq!(
        AlgorithmType::from_str("Snowflake").unwrap(),
        AlgorithmType::Snowflake
    );
    assert_eq!(
        AlgorithmType::from_str("UUIDV7").unwrap(),
        AlgorithmType::UuidV7
    );
    assert_eq!(
        AlgorithmType::from_str("Uuid7").unwrap(),
        AlgorithmType::UuidV7
    );
}

/// E2E-ALG-002: AlgorithmType::from_str 在未知算法时返回 InvalidAlgorithmType。
#[test]
fn e2e_algorithm_type_from_str_unknown_returns_error() {
    let result = AlgorithmType::from_str("redis");
    assert!(matches!(result, Err(CoreError::InvalidAlgorithmType(s)) if s == "redis"));
}

/// E2E-ALG-003: AlgorithmConfig::get_default_algorithm 解析配置字符串为枚举。
#[test]
fn e2e_algorithm_config_get_default_algorithm_parses_string() {
    let mut cfg = AlgorithmConfig::default();
    cfg.default = "snowflake".to_string();
    assert_eq!(cfg.get_default_algorithm(), AlgorithmType::Snowflake);

    cfg.default = "uuid_v7".to_string();
    assert_eq!(cfg.get_default_algorithm(), AlgorithmType::UuidV7);

    cfg.default = "uuidv4".to_string();
    assert_eq!(cfg.get_default_algorithm(), AlgorithmType::UuidV4);

    cfg.default = "segment".to_string();
    assert_eq!(cfg.get_default_algorithm(), AlgorithmType::Segment);
}

/// E2E-ALG-004: AlgorithmConfig::get_default_algorithm 在无效字符串时回退到 Segment。
#[test]
fn e2e_algorithm_config_get_default_algorithm_invalid_falls_back_to_segment() {
    let mut cfg = AlgorithmConfig::default();
    cfg.default = "invalid".to_string();
    assert_eq!(cfg.get_default_algorithm(), AlgorithmType::Segment);
}

// =============================================================================
// CoreError i18n 端到端
// =============================================================================

/// 串行化所有调用 `rust_i18n::set_locale` 的测试，避免并行 set_locale 竞态。
static E2E_LOCALE_LOCK: Mutex<()> = Mutex::new(());

/// E2E-I18N-001: 算法生成路径触发的 CoreError 通过 to_localized_string 翻译为用户可读消息。
///
/// 覆盖功能场景穷举分析第 2.7 节错误 i18n 行的"按 locale 翻译错误"。
/// 这是端到端跨层场景：Snowflake 时钟回拨产生 CoreError::ClockMovedBackward →
/// to_localized_string 在 en/zh-CN 下返回对应翻译。
///
/// 注：原计划通过 SnowflakeAlgorithm 私有字段模拟时钟回拨以触发真实错误，
/// 但 `last_timestamp` / `generate_id` 对模块外私有。修改源码仅为启用测试
/// 违反规则 6（外科手术式修改），因此改为直接构造 CoreError::ClockMovedBackward
/// 验证 i18n 翻译路径（key + args → 翻译消息）。snowflake.rs 单元测试
/// `test_generate_id_clock_backward_exceeds_threshold_returns_error` 已覆盖
/// 「算法 → 错误」路径，这里钉住「错误 → 翻译」路径。
#[test]
fn e2e_core_error_from_algorithm_to_localized_string_en_zh_cn() {
    let _lock = E2E_LOCALE_LOCK.lock().unwrap();

    let future_ts: u64 = 1_700_000_000_000;
    let err = CoreError::ClockMovedBackward {
        last_timestamp: future_ts,
    };

    let en_msg = err.to_localized_string("en");
    let zh_msg = err.to_localized_string("zh-CN");

    assert!(
        en_msg.contains("Clock moved backward") && en_msg.contains(&future_ts.to_string()),
        "E2E: en translation should mention error and timestamp, got: {}",
        en_msg
    );
    assert!(
        zh_msg.contains("时钟回拨") && zh_msg.contains(&future_ts.to_string()),
        "E2E: zh-CN translation should mention error and timestamp, got: {}",
        zh_msg
    );
}

/// E2E-I18N-002: 所有 24 个 CoreError 变体在 en 下都能产生非空翻译消息。
///
/// 覆盖功能场景穷举分析第 2.7 节错误 i18n 行的"24 个变体，i18n_key+i18n_args 双源"
/// 完整性检查——确保任何新增变体都有对应 i18n 翻译，避免运行时返回空字符串。
#[test]
fn e2e_core_error_all_variants_have_en_translation() {
    let _lock = E2E_LOCALE_LOCK.lock().unwrap();

    let variants: Vec<CoreError> = vec![
        CoreError::InvalidIdFormat("v".to_string()),
        CoreError::InvalidIdString("v".to_string()),
        CoreError::InvalidAlgorithmType("v".to_string()),
        CoreError::ClockMovedBackward { last_timestamp: 1 },
        CoreError::SequenceOverflow { timestamp: 1 },
        CoreError::SegmentExhausted { max_id: 1 },
        CoreError::DatabaseError("v".to_string()),
        CoreError::CacheError("v".to_string()),
        CoreError::ConfigurationError("v".to_string()),
        CoreError::AuthenticationError("v".to_string()),
        CoreError::RateLimitExceeded,
        CoreError::NotFound("v".to_string()),
        CoreError::WorkspaceDisabled("v".to_string()),
        CoreError::BizTagNotFound("v".to_string()),
        CoreError::ApiKeyDisabled,
        CoreError::ApiKeyExpired,
        CoreError::InvalidApiKeySignature,
        CoreError::EtcdError("v".to_string()),
        CoreError::ParseError("v".to_string()),
        CoreError::IoError("v".to_string()),
        CoreError::TimeoutError,
        CoreError::InternalError("v".to_string()),
        CoreError::InvalidInput("v".to_string()),
        CoreError::Unknown,
    ];

    assert_eq!(variants.len(), 24, "E2E: CoreError should have 24 variants");

    for (i, err) in variants.iter().enumerate() {
        let msg = err.to_localized_string("en");
        assert!(
            !msg.is_empty(),
            "E2E: variant #{} ({:?}) must have non-empty en translation",
            i,
            err.i18n_key()
        );
    }
}

/// E2E-I18N-003: 不支持的 locale 应回退到 en 而不 panic。
///
/// 覆盖功能场景穷举分析第 2.7 节错误 i18n 行的"不支持 locale 回退 en"。
#[test]
fn e2e_core_error_unsupported_locale_falls_back_to_en() {
    let _lock = E2E_LOCALE_LOCK.lock().unwrap();

    let err = CoreError::InvalidInput("test".to_string());
    let en_msg = err.to_localized_string("en");
    let fr_msg = err.to_localized_string("fr");
    let ja_msg = err.to_localized_string("ja");
    let bogus_msg = err.to_localized_string("xx-XX");

    assert_eq!(fr_msg, en_msg, "E2E: fr should fall back to en");
    assert_eq!(ja_msg, en_msg, "E2E: ja should fall back to en");
    assert_eq!(
        bogus_msg, en_msg,
        "E2E: bogus locale should fall back to en"
    );
}

/// E2E-I18N-004: i18n_key() 与 i18n_args() 一致性——key 返回非空，args 与变体匹配。
#[test]
fn e2e_core_error_i18n_key_and_args_consistency() {
    let err = CoreError::ClockMovedBackward { last_timestamp: 42 };
    let key = err.i18n_key();
    assert!(!key.is_empty());
    assert!(key.starts_with("error."));

    let args = err.i18n_args();
    assert!(
        args.iter()
            .any(|(k, v)| *k == "last_timestamp" && v == "42"),
        "E2E: ClockMovedBackward args should contain last_timestamp=42"
    );

    // 无参数变体
    let err = CoreError::RateLimitExceeded;
    assert!(err.i18n_args().is_empty());
    assert_eq!(err.i18n_key(), "error.rate_limit_exceeded");
}
