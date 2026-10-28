// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 核心支撑层端到端测试：Config 加载/校验/merge、Id 跨格式转换、
//! AlgorithmType 别名解析、CoreError i18n 翻译等跨子模块路径；与单元测试不重复。

use crate::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use crate::core::config::Config;
use crate::core::config::{AlgorithmConfig, ConfigError};
use crate::core::types::{AlgorithmType, CoreError, Id, IdBatch, IdFormat};
use std::str::FromStr;
use std::sync::Mutex;
use tempfile::tempdir;
use uuid::Uuid;

// Config 端到端

/// Config::load_from_file 完整链路：TOML 解析 + `${VAR}` 环境变量展开 + validate。
///
/// 用 `Config::default` 改字段后 `toml::to_string` 生成完整 TOML，避免手写遗漏必填字段。
#[tokio::test]
async fn e2e_config_load_from_file_with_env_expansion_and_validation() {
    let dir = tempdir().expect("E2E: tempdir should succeed");
    let file_path = dir.path().join("test_config.toml");

    // 字段值均与 default 不同，验证 load_from_file 真正读取而非回退默认值
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
    // 先设固定 url，便于随后替换为 ${VAR} 占位符
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

/// Config::load_from_file 在文件不存在时返回 FileNotFound（与读取 IO 错误可区分）。
#[tokio::test]
async fn e2e_config_load_from_file_missing_returns_file_not_found() {
    let result = Config::load_from_file("/nonexistent/path/to/config.toml");
    match result {
        Err(ConfigError::FileNotFound(msg)) => {
            assert!(
                msg.contains("/nonexistent/path/to/config.toml"),
                "E2E: FileNotFound should name the missing path, got: {}",
                msg
            );
        }
        // 断言消息不打印结果本体：`load_from_file` 的返回被 CodeQL
        // rust/cleartext-logging 视为含密数据，输出到断言消息即告警。
        _ => panic!("E2E: expected FileNotFound, got a different result"),
    }
}

/// Config::load_from_file 在 TOML 格式错误时返回 InvalidValue。
#[tokio::test]
async fn e2e_config_load_from_file_invalid_toml_returns_invalid_value() {
    let dir = tempdir().expect("E2E: tempdir");
    let file_path = dir.path().join("invalid.toml");
    std::fs::write(&file_path, "not = valid = toml = syntax =").unwrap();

    let result = Config::load_from_file(file_path.to_str().unwrap());
    assert!(
        matches!(result, Err(ConfigError::InvalidValue(_))),
        "E2E: invalid TOML should return InvalidValue, got a different result"
    );
}

/// http_port=0 时 validate 返回 InvalidValue（端口 1-65535 边界）。
#[test]
fn e2e_config_validate_http_port_zero_fails() {
    let mut config = Config::default();
    config.app.http_port = 0;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// dc_id 超出位宽（默认 datacenter_id_bits=3，上限 7）时 validate 返回 InvalidValue。
#[test]
fn e2e_config_validate_dc_id_over_width_fails() {
    let mut config = Config::default();
    config.app.dc_id = 32;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// dc_id 在位宽内时通过；位宽放宽（≥5 位）后 31 也合法。
#[test]
fn e2e_config_validate_dc_id_within_width_passes() {
    let mut config = Config::default();
    config.app.dc_id = 7;
    assert!(config.validate().is_ok());

    let mut wide = Config::default();
    wide.algorithm.snowflake.datacenter_id_bits = 5;
    wide.app.dc_id = 31;
    assert!(
        wide.validate().is_ok(),
        "位宽放宽到 5 位后 dc_id=31 必须合法"
    );
}

/// Config::validate 在 min_connections > max_connections 时失败。
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

/// Config::validate 在 rate_limit 启用但 burst_size > 10×rps 时失败。
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

/// Config::validate 在 snowflake 总位数 ≥ 64 时失败。
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

/// Config::validate 在 batch_generate.max_batch_size > 10000 时失败。
#[test]
fn e2e_config_validate_batch_size_over_10000_fails() {
    let mut config = Config::default();
    config.batch_generate.max_batch_size = 10001;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// Config::validate 在 batch_generate.max_batch_size == 10000 时通过（边界）。
#[test]
fn e2e_config_validate_batch_size_10000_passes() {
    let mut config = Config::default();
    config.batch_generate.max_batch_size = 10000;
    assert!(config.validate().is_ok());
}

/// Config::validate 在 algorithm.default 不是合法值时失败。
#[test]
fn e2e_config_validate_invalid_default_algorithm_fails() {
    let mut config = Config::default();
    config.algorithm.default = "redis".to_string();
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// Config::validate 在 switch_threshold 越界时失败。
#[test]
fn e2e_config_validate_switch_threshold_out_of_range_fails() {
    let mut config = Config::default();
    config.algorithm.segment.switch_threshold = 1.5;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidValue(_))
    ));
}

/// Config::merge 以默认值为哨兵：other 中显式设为默认值的字段不覆盖 base（已知限制）。
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

/// Config::merge 在 other 字段等于默认值时不覆盖 base。
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

// Id 类型跨格式端到端

/// 真实 Snowflake 生成的 ID 往返多种格式：to_string / to_hex / to_base36 / to_prefixed。
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
        dc: 0,
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

/// UUID v7 字符串经 Id::from_string 解析后 roundtrip（36 位含 '-'）。
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

/// UUID v4 字符串通过 Id::from_string 解析后保持 v4 版本位。
#[test]
fn e2e_id_from_string_uuid_v4_preserves_version() {
    let uuid = Uuid::new_v4();
    let id = Id::from_string(&uuid.to_string()).expect("E2E: parse UUID v4 string");
    let back = id.to_uuid_v8();
    assert_eq!(back.get_version(), Some(uuid::Version::Random));
}

/// 空串/非数字/非 UUID 时 Id::from_string 返回 InvalidIdString。
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

/// IdBatch::from_u64s 与 IdBatch::new 互转。
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

// AlgorithmType 别名解析端到端

/// AlgorithmType::from_str 接受所有名称与大小写变体（大小写不敏感）。
///
/// uuid_v7 / uuid_v4 系列已反转为真算法 UuidV7 / UuidV4（破坏性变更，语义见 CHANGELOG）。
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
        AlgorithmType::from_str("uuid_v8").unwrap(),
        AlgorithmType::UuidV8
    );

    // uuid_v8 自身别名
    assert_eq!(
        AlgorithmType::from_str("uuidv8").unwrap(),
        AlgorithmType::UuidV8
    );
    assert_eq!(
        AlgorithmType::from_str("uuid8").unwrap(),
        AlgorithmType::UuidV8
    );
    // v7 组：指向真算法 UuidV7
    assert_eq!(
        AlgorithmType::from_str("uuid_v7").unwrap(),
        AlgorithmType::UuidV7
    );
    assert_eq!(
        AlgorithmType::from_str("uuidv7").unwrap(),
        AlgorithmType::UuidV7
    );
    assert_eq!(
        AlgorithmType::from_str("uuid7").unwrap(),
        AlgorithmType::UuidV7
    );
    // v4 组：指向真算法 UuidV4
    assert_eq!(
        AlgorithmType::from_str("uuid_v4").unwrap(),
        AlgorithmType::UuidV4
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

/// AlgorithmType::from_str 在未知算法时返回 InvalidAlgorithmType。
#[test]
fn e2e_algorithm_type_from_str_unknown_returns_error() {
    let result = AlgorithmType::from_str("redis");
    assert!(matches!(result, Err(CoreError::InvalidAlgorithmType(s)) if s == "redis"));
}

/// AlgorithmConfig::get_default_algorithm 解析配置字符串为枚举。
#[test]
fn e2e_algorithm_config_get_default_algorithm_parses_string() {
    let mut cfg = AlgorithmConfig {
        default: "snowflake".to_string(),
        ..Default::default()
    };
    assert_eq!(cfg.get_default_algorithm(), AlgorithmType::Snowflake);

    cfg.default = "uuid_v8".to_string();
    assert_eq!(cfg.get_default_algorithm(), AlgorithmType::UuidV8);

    cfg.default = "uuidv8".to_string();
    assert_eq!(cfg.get_default_algorithm(), AlgorithmType::UuidV8);

    cfg.default = "segment".to_string();
    assert_eq!(cfg.get_default_algorithm(), AlgorithmType::Segment);
}

/// AlgorithmConfig::get_default_algorithm 在无效字符串时回退到 Segment。
#[test]
fn e2e_algorithm_config_get_default_algorithm_invalid_falls_back_to_segment() {
    let cfg = AlgorithmConfig {
        default: "invalid".to_string(),
        ..Default::default()
    };
    assert_eq!(cfg.get_default_algorithm(), AlgorithmType::Segment);
}

// CoreError i18n 端到端

/// 串行化相邻的环境变量/locale 敏感测试(与 core::i18n::test_support 同一意图),避免并行竞态。
static E2E_LOCALE_LOCK: Mutex<()> = Mutex::new(());

/// CoreError 经 to_localized_string 在 en/zh-CN 下翻译（端到端跨层）。
///
/// 直接构造 ClockMovedBackward 钉「错误 → 翻译」路径；「算法 → 错误」路径由
/// snowflake.rs 单元测试 test_generate_id_clock_backward_exceeds_threshold_returns_error 覆盖。
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

/// 所有 24 个 CoreError 变体在 en 下都有非空翻译（防新增变体漏 i18n）。
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
        CoreError::DatabaseError("v".to_string(), None),
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
        CoreError::EtcdError("v".to_string(), None),
        CoreError::ParseError("v".to_string()),
        CoreError::IoError("v".to_string(), None),
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

/// 不支持的 locale 回退到 en 而不 panic。
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

/// E2E-I18N-004: i18n_key 与 i18n_args 一致性——key 返回非空，args 与变体匹配。
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
