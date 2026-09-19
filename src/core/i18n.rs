// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Internationalization (Fluent/ICU i18n — unify-rust-i18n 基线).
//!
//! wraps `fluent-bundle`(concurrent 双束)+ `unic-langid`,替换原 rust-i18n
//! YAML 后端:
//! - FTL 资源 `locales/{en,zh}/messages.ftl` 经 `include_str!` 编译期内嵌
//!   (内嵌与磁盘是同一份文件,结构上不可能漂移,见守卫测试
//!   `test_embedded_ftl_matches_locales_dir`);
//! - 首次访问时解析并缓存进 `OnceLock<FluentBundle<FluentResource>>`
//!   (`fluent_bundle::concurrent::FluentBundle` 是 Send+Sync,可入 static;
//!   非 concurrent 的 `FluentBundle` 含 RefCell,不能放 static);
//! - 运行时进程级 locale 切换经 [`init_i18n`](`t!` 宏的全局 locale 来源);
//! - 每请求 locale 翻译经 [`translate_with_locale`] 系列——被 HTTP handlers
//!   用于按 `Accept-Language` 协商出的 `Locale` 翻译错误消息,不触碰全局
//!   locale 状态(并发安全)。
//!
//! # 键名约定(与 rust-i18n 时代的点分键兼容)
//!
//! FTL 消息标识不允许 `.`;源键 `a.b.c_d` 与 FTL id `a-b-c_d` 一一对应
//! (查找时把 `.` 替换为 `-`;两份 yml 中没有任何含 `-` 的键,映射双射,
//! 见守卫测试)。对外 API(`i18n_key()`、`t!` 调用点、FTL 键齐性测试的
//! 语义)全部沿用点分键,零调用点改动。
//!
//! # 回退语义(与原 rust-i18n 行为一致)
//!
//! 请求 locale → `en` 束 → 键本身(永不 panic、永不返回空串);未知 locale
//! ("fr"/"ja"/"xx-XX"/"") 一律落到 `en` 束。

use std::borrow::Cow;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

/// 全局 locale 存储:`0 = en`(默认),`1 = zh`。
///
/// 结果域只有 en/zh 两个值(unify-rust-i18n 合规标准:禁止第三语言),
/// 因此用原子量即可,无需 RwLock<String>。
static GLOBAL_LANG: AtomicU8 = AtomicU8::new(0);

const LANG_EN: u8 = 0;
const LANG_ZH: u8 = 1;

/// Initialize the i18n system with the given locale.
///
/// Must be called once at startup, before any `t!()` lookup that depends
/// on a non-default locale. 接受 "en" / "zh-CN"(大小写不敏感,"zh*" 归一
/// 为 zh);未知取值按原 rust-i18n 语义回退 "en"。
///
/// # Example
/// ```ignore
/// nebulaid::core::i18n::init_i18n("en");
/// ```
pub fn init_i18n(locale: &str) {
    let lang = if locale
        .trim()
        .get(..2)
        .map(|p| p.eq_ignore_ascii_case("zh"))
        .unwrap_or(false)
    {
        LANG_ZH
    } else {
        LANG_EN
    };
    GLOBAL_LANG.store(lang, Ordering::Release);
}

/// Current process default locale as its canonical string ("en" / "zh-CN").
///
/// 供测试守卫保存/恢复全局 locale,以及需要读取(而非改写)全局 locale
/// 的调用方使用。
pub fn current_locale() -> String {
    if GLOBAL_LANG.load(Ordering::Acquire) == LANG_ZH {
        "zh-CN".to_string()
    } else {
        "en".to_string()
    }
}

fn current_lang() -> &'static str {
    if GLOBAL_LANG.load(Ordering::Acquire) == LANG_ZH {
        "zh"
    } else {
        "en"
    }
}

/// 请求 locale 字符串 → 束语言("en" / "zh")。
///
/// "zh-CN" / "zh" / "zh-Hans"(大小写不敏感)→ zh;其余(含 en 变体、
/// 未知、空串)→ en。HTTP 中间件的 `Locale::as_str()` 输出("en"/"zh-CN")
/// 与 `resolve_locale` 的输出都按此归一。
fn lang_of(locale: &str) -> &'static str {
    if locale
        .trim()
        .get(..2)
        .map(|p| p.eq_ignore_ascii_case("zh"))
        .unwrap_or(false)
    {
        "zh"
    } else {
        "en"
    }
}

/// 源点分键 → FTL 消息标识:`a.b.c_d` → `a-b-c_d`。
fn ftl_id(dotted_key: &str) -> String {
    dotted_key.replace('.', "-")
}

/// Translate a key under the given locale, without pattern arguments.
///
/// Looks up the key in the locale's FTL bundle. If the key is missing for
/// the requested locale, falls back to the crate default locale (`en`)
/// bundle. If still missing, returns the key itself.
///
/// Unlike `t!` (which consults the process-wide locale set by
/// [`init_i18n`]), this function does **not** mutate or read global state
/// and is safe for concurrent use across requests with different
/// `Accept-Language` headers.
pub fn translate_with_locale(locale: &str, key: &str) -> String {
    translate_with_locale_cow(locale, key).into_owned()
}

/// Zero-copy-flavored translation lookup returning `Cow<'static, str>`.
///
/// 历史注记:rust-i18n 时代命中编译期内嵌静态串时可返回
/// `Cow::Borrowed`;Fluent 引擎总是按 pattern 格式化求值,输出恒为
/// 新分配的 `String`(`Cow::Owned`)。签名与"返回可读翻译"的语义
/// 原样保留,调用方无需改动;按值读取的调用方可以继续 `as_ref()`,
/// 需要 `String` 的调用方 `into_owned()`。
pub fn translate_with_locale_cow(locale: &str, key: &str) -> Cow<'static, str> {
    Cow::Owned(lookup(lang_of(locale), key, Args::None))
}

/// Translate a key under the given locale, substituting named variables.
///
/// `args` is a slice of `(variable_name, value)` pairs; each `{ $name }`
/// variable in the FTL pattern is resolved with the corresponding value.
/// Empty `args` skips the argument-passing pass for efficiency.
///
/// Like [`translate_with_locale`], this function does not touch global
/// locale state and is safe for concurrent per-request use.
pub fn translate_with_locale_args(locale: &str, key: &str, args: &[(&str, String)]) -> String {
    lookup(lang_of(locale), key, Args::Owned(args))
}

/// Borrow-friendly variant of [`translate_with_locale_args`] accepting
/// `Cow<str>` values.
///
/// 被 `CoreError::to_localized_string` 使用:`i18n_args()` 对 String 载荷
/// 变体返回 `Cow::Borrowed`(零克隆),数值载荷变体返回 `Cow::Owned`
/// (恰一次数字文本分配)。
pub fn translate_with_locale_args_cow(
    locale: &str,
    key: &str,
    args: &[(&str, Cow<'_, str>)],
) -> String {
    lookup(lang_of(locale), key, Args::Cow(args))
}

/// `t!` 宏的运行时支撑:以进程默认 locale 翻译(当前语言 → en → 键本身)。
pub fn global_translate(key: &str, args: &[(&str, String)]) -> String {
    lookup(current_lang(), key, Args::Owned(args))
}

/// OpenAPI 文档运行时本地化用的「命中才译」查询(openapi.rs 专用)。
///
/// 与 [`lookup`]「未命中回退键本身」的语义不同,这里显式区分命中与未命中:
/// `ftl_id`(FTL 消息标识,如 `post-generate-200`,不做点分转换)在请求语言束
/// (未命中再查 en 束)中存在时返回其渲染结果,不存在返回 `None`——调用方
/// 据此保留 utoipa 静态注册的英文规范串,OpenAPI 契约结构不变。
pub fn translate_if_present(locale: &str, ftl_id: &str) -> Option<String> {
    let lang = lang_of(locale);
    format_from_bundle(lang, ftl_id, &Args::None)
        .or_else(|| format_from_bundle("en", ftl_id, &Args::None))
}

enum Args<'a> {
    None,
    Owned(&'a [(&'a str, String)]),
    Cow(&'a [(&'a str, Cow<'a, str>)]),
}

/// 查找链:请求语言束 → "en" 束 → 键本身(不得 panic)。
fn lookup(lang: &str, dotted_key: &str, args: Args<'_>) -> String {
    let id = ftl_id(dotted_key);
    format_from_bundle(lang, &id, &args)
        .or_else(|| format_from_bundle("en", &id, &Args::None))
        .unwrap_or_else(|| dotted_key.to_string())
}

// ============================================================================
// Fluent bundle management(dbnexus catalog.rs concurrent 模式)
// ============================================================================

/// Cached concurrent Fluent bundles (thread-safe, built once on first access).
static EN_BUNDLE: OnceLock<FluentBundle<FluentResource>> = OnceLock::new();
static ZH_BUNDLE: OnceLock<FluentBundle<FluentResource>> = OnceLock::new();

/// English (en) Fluent resource — 编译期内嵌 locales/en/messages.ftl。
const EN_FTL: &str = include_str!("../../locales/en/messages.ftl");
/// Chinese (zh) Fluent resource — 编译期内嵌 locales/zh/messages.ftl。
const ZH_FTL: &str = include_str!("../../locales/zh/messages.ftl");

/// Format a message from the Fluent catalog for the given language.
fn format_from_bundle(lang: &str, key: &str, args: &Args<'_>) -> Option<String> {
    let bundle = match lang {
        "zh" => ZH_BUNDLE.get_or_init(build_zh_bundle),
        _ => EN_BUNDLE.get_or_init(build_en_bundle),
    };

    let msg = bundle.get_message(key)?;
    let pattern = msg.value()?;

    let mut fluent_args = FluentArgs::new();
    match args {
        Args::None => {}
        Args::Owned(pairs) => {
            for (name, value) in pairs.iter() {
                fluent_args.set(*name, FluentValue::from(value.clone()));
            }
        }
        Args::Cow(pairs) => {
            for (name, value) in pairs.iter() {
                fluent_args.set(*name, FluentValue::from(value.to_string()));
            }
        }
    }

    let mut errors = vec![];
    let result = bundle.format_pattern(pattern, Some(&fluent_args), &mut errors);
    Some(result.to_string())
}

fn build_en_bundle() -> FluentBundle<FluentResource> {
    let resource = FluentResource::try_new(EN_FTL.to_string()).unwrap_or_else(|e| e.0);
    let langid: LanguageIdentifier = "en".parse().expect("'en' is a valid language identifier");
    let mut bundle = FluentBundle::new_concurrent(vec![langid]);
    // 避免输出含 Unicode 隔离符(与 rust-i18n 时代输出逐字节兼容)
    bundle.set_use_isolating(false);
    bundle
        .add_resource(resource)
        .expect("EN resources should add without conflict");
    bundle
}

fn build_zh_bundle() -> FluentBundle<FluentResource> {
    let resource = FluentResource::try_new(ZH_FTL.to_string()).unwrap_or_else(|e| e.0);
    let langid: LanguageIdentifier = "zh".parse().expect("'zh' is a valid language identifier");
    let mut bundle = FluentBundle::new_concurrent(vec![langid]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(resource)
        .expect("ZH resources should add without conflict");
    bundle
}

#[cfg(test)]
pub(crate) mod test_support {
    /// 串行化改写进程默认 locale 的测试(i18n.rs / types::error.rs /
    /// server::handlers::helpers.rs 共用),避免并行测试竞态。
    static TEST_LOCALE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(crate) fn lock() -> std::sync::MutexGuard<'static, ()> {
        match TEST_LOCALE_LOCK.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Save and restore the global locale around tests that may touch it.
    struct LocaleGuard {
        saved: String,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl LocaleGuard {
        fn new() -> Self {
            Self {
                saved: current_locale(),
                _lock: test_support::lock(),
            }
        }
    }

    impl Drop for LocaleGuard {
        fn drop(&mut self) {
            init_i18n(&self.saved);
        }
    }

    #[test]
    fn test_translate_with_locale_en() {
        assert_eq!(
            translate_with_locale("en", "error.unknown"),
            "Unknown error"
        );
    }

    #[test]
    fn test_translate_with_locale_zh_cn() {
        assert_eq!(translate_with_locale("zh-CN", "error.unknown"), "未知错误");
    }

    #[test]
    fn test_translate_with_locale_args_en() {
        assert_eq!(
            translate_with_locale_args(
                "en",
                "error.invalid_input",
                &[("value", "negative".to_string())]
            ),
            "Invalid input: negative"
        );
    }

    #[test]
    fn test_translate_with_locale_args_zh_cn() {
        assert_eq!(
            translate_with_locale_args(
                "zh-CN",
                "error.invalid_input",
                &[("value", "negative".to_string())]
            ),
            "无效输入：negative"
        );
    }

    #[test]
    fn test_config_defaults_because_missing_message_interpolates_path_en() {
        assert_eq!(
            translate_with_locale_args(
                "en",
                "log.main.config_defaults_because_missing",
                &[("path", "config/config.toml".to_string())]
            ),
            "No --config was given and 'config/config.toml' does not exist; \
             starting with built-in default configuration"
        );
    }

    #[test]
    fn test_config_defaults_because_missing_message_interpolates_path_zh_cn() {
        assert_eq!(
            translate_with_locale_args(
                "zh-CN",
                "log.main.config_defaults_because_missing",
                &[("path", "config/config.toml".to_string())]
            ),
            "未指定 --config 且配置文件“config/config.toml”不存在，正在使用内置默认配置启动"
        );
    }

    #[test]
    fn test_translate_with_locale_missing_key_returns_key() {
        // Non-existent key returns the key itself (no panic, no empty string)
        assert_eq!(
            translate_with_locale("en", "error.nonexistent_key_xyz_123"),
            "error.nonexistent_key_xyz_123"
        );
    }

    #[test]
    fn test_translate_with_locale_unsupported_locale_falls_back() {
        // Unsupported locale falls back to the "en" bundle
        let s = translate_with_locale("fr", "error.unknown");
        assert_eq!(s, "Unknown error");
    }

    #[test]
    fn test_translate_with_locale_args_empty_args() {
        assert_eq!(
            translate_with_locale_args("en", "error.unknown", &[]),
            "Unknown error"
        );
    }

    #[test]
    fn test_translate_with_locale_args_named_arg() {
        assert_eq!(
            translate_with_locale_args(
                "en",
                "error.clock_moved_backward",
                &[("last_timestamp", "123".to_string())]
            ),
            "Clock moved backward, last timestamp: 123"
        );
    }

    #[test]
    fn test_translate_with_locale_does_not_mutate_global_locale() {
        let _g = LocaleGuard::new();
        init_i18n("en");
        // Translate in zh-CN
        let _ = translate_with_locale("zh-CN", "error.unknown");
        // Global locale should remain "en" — no race condition risk
        assert_eq!(current_locale(), "en");
    }

    #[test]
    fn test_translate_with_locale_concurrent_safety() {
        // Simulate concurrent calls with different locales — each call should
        // return its own locale's translation, regardless of call ordering.
        let en = translate_with_locale("en", "error.unknown");
        let zh = translate_with_locale("zh-CN", "error.unknown");
        let en2 = translate_with_locale("en", "error.unknown");
        let zh2 = translate_with_locale("zh-CN", "error.unknown");

        assert_eq!(en, "Unknown error");
        assert_eq!(zh, "未知错误");
        assert_eq!(en2, "Unknown error");
        assert_eq!(zh2, "未知错误");
    }

    /// `translate_with_locale_cow` returns the translation as a `Cow`
    /// (Fluent 引擎输出恒为 `Cow::Owned`,见函数文档)且值正确。
    #[test]
    fn test_translate_with_locale_cow_borrowed_for_static_strings() {
        let cow = translate_with_locale_cow("en", "error.unknown");
        assert_eq!(cow.as_ref(), "Unknown error");
    }

    /// 原 YAML 值中的 `\n` 转义(真实换行)在 FTL 中以 StringLiteral
    /// `{ "\n" }` 承载,必须渲染回真实换行(含续行前导空格)。
    #[test]
    fn test_multiline_fatal_message_preserves_newlines() {
        let en = translate_with_locale("en", "log.main.fatal_api_key_auth_requires_database");
        assert!(
            en.contains("database connection.\nNebula ID requires"),
            "换行必须保留,实际: {en:?}"
        );
        assert!(en.contains("\n  - database.url"), "续行前导空格必须保留");

        let zh = translate_with_locale("zh-CN", "log.main.fatal_api_key_auth_requires_database");
        assert!(
            zh.contains("需要数据库连接。\nNebula ID 需要数据库"),
            "换行必须保留,实际: {zh:?}"
        );
    }

    /// `translate_with_locale_args_cow` accepts `Cow<str>` values (borrowed
    /// or owned) and produces the same output as the `String`-based variant.
    #[test]
    fn test_translate_with_locale_args_cow_equivalent_to_owned() {
        let owned = translate_with_locale_args(
            "en",
            "error.invalid_input",
            &[("value", "negative".to_string())],
        );
        let borrowed = translate_with_locale_args_cow(
            "en",
            "error.invalid_input",
            &[("value", Cow::Borrowed("negative"))],
        );
        assert_eq!(owned, borrowed);
        assert_eq!(owned, "Invalid input: negative");
    }

    /// Empty args path returns the translation verbatim.
    #[test]
    fn test_translate_with_locale_args_cow_empty_args() {
        assert_eq!(
            translate_with_locale_args_cow("en", "error.unknown", &[]),
            "Unknown error"
        );
    }

    /// 未知语言("ar")取 en 束(unify-rust-i18n 守卫 §5.3)。
    #[test]
    fn test_unknown_language_falls_back_to_en_bundle() {
        assert_eq!(
            translate_with_locale("ar", "error.unknown"),
            "Unknown error"
        );
        assert_eq!(
            translate_with_locale("zh-TW", "error.unknown"),
            "未知错误",
            "zh* 全部归一 zh 束"
        );
        assert_eq!(
            translate_with_locale("", "error.unknown"),
            "Unknown error",
            "空 locale 回退 en"
        );
    }

    /// init_i18n 全局 locale 生效于 t! 运行时支撑,且未知取值回退 en。
    #[test]
    fn test_init_i18n_and_global_translate() {
        let _g = LocaleGuard::new();
        init_i18n("zh-CN");
        assert_eq!(current_locale(), "zh-CN");
        assert_eq!(global_translate("error.unknown", &[]), "未知错误");
        assert_eq!(
            global_translate("error.invalid_input", &[("value", "x".to_string())]),
            "无效输入：x"
        );
        // 未知 locale → en
        init_i18n("fr");
        assert_eq!(current_locale(), "en");
        assert_eq!(global_translate("error.unknown", &[]), "Unknown error");
    }

    /// 守卫一(en/zh FTL 键齐性)+ 守卫二(内嵌 == 磁盘同步):
    /// include_str! 内嵌常量与 locales/{en,zh}/messages.ftl 是同一份文件,
    /// 这里仍显式断言防未来误改为内嵌字面量;键集合经 `-`→`.` 映射后必须
    /// 完全一致(单侧缺键只会在该语言静默回退,本仓库实际发生过)。
    #[test]
    fn test_embedded_ftl_matches_locales_dir_and_key_sets_are_identical() {
        fn message_keys(ftl: &str) -> Vec<String> {
            ftl.lines()
                .filter_map(|line| line.split_once(" = "))
                .map(|(key, _)| key.trim().replace('-', "."))
                .collect()
        }

        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let en_disk = std::fs::read_to_string(
            std::path::Path::new(manifest_dir).join("locales/en/messages.ftl"),
        )
        .expect("locales/en/messages.ftl must exist on disk");
        let zh_disk = std::fs::read_to_string(
            std::path::Path::new(manifest_dir).join("locales/zh/messages.ftl"),
        )
        .expect("locales/zh/messages.ftl must exist on disk");

        // 守卫二:内嵌 == 磁盘(当前经 include_str! 结构性成立)
        assert_eq!(EN_FTL, en_disk, "EN 内嵌 FTL 与磁盘文件不同步");
        assert_eq!(ZH_FTL, zh_disk, "ZH 内嵌 FTL 与磁盘文件不同步");

        // 守卫一:键集合一致
        let mut en = message_keys(EN_FTL);
        let mut zh = message_keys(ZH_FTL);
        assert!(
            en.len() > 300 && zh.len() > 300,
            "键数量异常(en={}、zh={}),解析逻辑可能失效",
            en.len(),
            zh.len()
        );
        en.sort();
        zh.sort();
        let only_en: Vec<&String> = en.iter().filter(|k| !zh.contains(k)).collect();
        let only_zh: Vec<&String> = zh.iter().filter(|k| !en.contains(k)).collect();
        assert!(
            only_en.is_empty() && only_zh.is_empty(),
            "locale 键集不对齐:仅 en 有 {only_en:?};仅 zh 有 {only_zh:?}"
        );
    }

    /// 新增 FTL 键(en/zh 双侧)必须在两束中都能取到,防止守卫解析逻辑
    /// 自身失效导致键齐性测试空转。
    #[test]
    fn test_t013_dynamic_error_detail_keys_present_both_bundles() {
        let id_args = Args::Owned(&[("id", "ws-1".to_string())]);
        assert_eq!(
            format_from_bundle("en", "error-detail-workspace_not_found", &id_args).as_deref(),
            Some("Workspace not found: ws-1")
        );
        assert_eq!(
            format_from_bundle("zh", "error-detail-workspace_not_found", &id_args).as_deref(),
            Some("工作空间未找到：ws-1")
        );

        let args = Args::Owned(&[
            ("path", "config/config.toml".to_string()),
            ("reason", "boom".to_string()),
        ]);
        assert_eq!(
            format_from_bundle("en", "error-main-config_load_failed", &args).as_deref(),
            Some("Failed to load configuration from 'config/config.toml': boom")
        );
        assert_eq!(
            format_from_bundle("zh", "error-main-config_load_failed", &args).as_deref(),
            Some("从 'config/config.toml' 加载配置失败：boom")
        );
    }
}
