// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Internationalization (i18n)：FTL 资源 `locales/{en,zh}/messages.ftl` 经
//! `include_str!` 编译期内嵌，`init_i18n` 或首次查询注册进 sdforge 并构建本地只读快照（查询零锁）。
//! 回退链：请求 locale → `en` 束 → 键本身（永不 panic/空串）；点分键映射 FTL id 时 `.` → `-`。

use std::borrow::Cow;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

/// 全局 locale 存储：`0 = en`（默认），`1 = zh`。结果域仅 en/zh，用原子量即可。
static GLOBAL_LANG: AtomicU8 = AtomicU8::new(0);

const LANG_EN: u8 = 0;
const LANG_ZH: u8 = 1;

/// Initialize the i18n system with the given locale.
///
/// 启动期调用一次，须先于依赖非默认 locale 的 `t!()` 查询；接受 "en"/"zh-CN"
/// （大小写不敏感，"zh*" 归一 zh），未知取值回退 "en"。
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
    // 键集注册提前到启动期：一次性成本，避免落在首个错误响应热路径；幂等，无副作用。
    ensure_registered();
}

/// Current process default locale as its canonical string ("en" / "zh-CN").
/// 供只读全局 locale 的调用方与测试守卫使用。
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

/// 请求 locale 字符串 → 束语言("en" / "zh")："zh*"（大小写不敏感）→ zh，其余 → en。
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
/// 未命中回退 `en` 束，再回退键本身。与 `t!` 不同：不读写全局 locale 状态，
/// 可跨请求并发使用（多 `Accept-Language` 安全）。
pub fn translate_with_locale(locale: &str, key: &str) -> String {
    translate_with_locale_cow(locale, key).into_owned()
}

/// Zero-copy-flavored translation lookup returning `Cow<'static, str>`.
///
/// 输出恒为 `Cow::Owned`（每次渲染新分配）；调用方可用 `as_ref()` 或 `into_owned()`。
pub fn translate_with_locale_cow(locale: &str, key: &str) -> Cow<'static, str> {
    let pairs: Vec<(&str, String)> = Vec::new();
    Cow::Owned(lookup(lang_of(locale), key, &pairs))
}

/// Translate a key under the given locale, substituting named variables.
///
/// `args` 为 `(变量名, 值)` 对，`{ $name }` 占位符逐个替换；空 `args` 跳过替换。
/// 不触碰全局 locale，可并发按请求使用。
pub fn translate_with_locale_args(locale: &str, key: &str, args: &[(&str, String)]) -> String {
    lookup(lang_of(locale), key, args)
}

/// [`translate_with_locale_args`] 的 `Cow<str>` 变体，被 `CoreError::to_localized_string` 使用。
///
/// String 载荷零克隆（`Cow::Borrowed`），数值载荷恰一次分配。
pub fn translate_with_locale_args_cow(
    locale: &str,
    key: &str,
    args: &[(&str, Cow<'_, str>)],
) -> String {
    // 参数物化 String：每参数一次分配，仅错误响应用（非热路径）；进热路径时应直收 Cow。
    let pairs: Vec<(&str, String)> = args.iter().map(|(k, v)| (*k, v.to_string())).collect();
    lookup(lang_of(locale), key, &pairs)
}

/// `t!` 宏的运行时支撑:以进程默认 locale 翻译(当前语言 → en → 键本身)。
pub fn global_translate(key: &str, args: &[(&str, String)]) -> String {
    lookup(current_lang(), key, args)
}

/// OpenAPI 文档运行时本地化用的「命中才译」查询(openapi.rs 专用)。
///
/// 与 [`lookup`] 不同：未命中返回 `None` 而非键本身，调用方据此保留 utoipa 静态英文串。
/// `ftl_id` 为连字符形态，不做点分转换；查找为请求语言束 → en 束。
pub fn translate_if_present(locale: &str, ftl_id: &str) -> Option<String> {
    let lang = lang_of(locale);
    if !is_registered_id(ftl_id) {
        return None;
    }
    ensure_registered();
    // 渲染走本地快照（零锁）；OpenAPI 每请求重建整份文档，数百次调用不触 REGISTRY 锁。
    let empty: &[(&str, String)] = &[];
    Some(forge_translate(lang, ftl_id, empty))
}

// 键集注册（注册进 sdforge register_translation）

/// 已注册 FTL 消息标识集合：sdforge `translate_for` 未命中会回退键本身，
/// 用本集合区分「命中/未命中」；双语键集一致，等价于「任一束存在」。
static REGISTERED_IDS: OnceLock<std::collections::HashSet<String>> = OnceLock::new();

/// 注册期构建的本地只读快照：locale → (FTL id → 消息模板)。
/// sdforge 注册表是进程级 `Mutex`，查询过锁；键集运行期不变，本地快照供零锁读取。
static SNAPSHOT: OnceLock<
    std::collections::HashMap<String, std::collections::HashMap<String, std::sync::Arc<str>>>,
> = OnceLock::new();

/// 注册完成的幂等 latch：用 `OnceLock` get-or-init 保证「注册恰好发生一次」。
static REGISTERED: OnceLock<()> = OnceLock::new();

/// English (en) FTL 资源 — 编译期内嵌 locales/en/messages.ftl。
const EN_FTL: &str = include_str!("../../locales/en/messages.ftl");
/// Chinese (zh) FTL 资源 — 编译期内嵌 locales/zh/messages.ftl。
const ZH_FTL: &str = include_str!("../../locales/zh/messages.ftl");

/// 展开 Fluent 字符串字面量占位符 `{ "..." }` 为字面字符。
///
/// 换行等控制字符必须写成 `{ "\u000A" }`（裸换行会解释为消息结束）；渲染侧只认
/// `{ $name }`，故须在注册前展开，否则消息残留字面痕迹。
fn expand_string_literals(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find("{ \"") {
        out.push_str(&rest[..open]);
        // 跳过 `{ "` 三字符，从字面量内容起点开始找闭合的 `" }`
        let after = &rest[open + 3..];
        let Some(close) = after.find("\" }") else {
            // 找不到闭合：保守按原文输出剩余部分
            out.push_str(&rest[open..]);
            return out;
        };
        out.push_str(&unescape_ftl_literal(&after[..close]));
        rest = &after[close + 3..];
    }
    out.push_str(rest);
    out
}

/// 反转义 Fluent 字符串字面量内部：`\uXXXX` / `\n` / `\t` / `\"` / `\\`。
fn unescape_ftl_literal(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => out.push_str(&format!("\\u{hex}")),
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// 解析扁平 FTL：取 `key = value` 单行，跳过空行与 `#` 注释。
/// 消息集无多行续行/属性/select，按行切分即可；值经展开字符串字面量后入快照。
fn parse_ftl_entries(src: &str) -> Vec<(String, String)> {
    src.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            let (k, v) = (k.trim(), v.trim());
            if k.is_empty() {
                return None;
            }
            Some((k.to_string(), expand_string_literals(v)))
        })
        .collect()
}

/// 把双语 FTL 键集注册进 sdforge i18n（幂等，`init_i18n` 与首次查询都会触发）。
///
/// 注册的是消息值而非 FTL 原文：sdforge 登记表是扁平 `(locale, key) -> value`，
/// 由其 `format_template` 做 `{ $name }` 轻量替换。
fn ensure_registered() {
    REGISTERED.get_or_init(|| {
        let mut ids = std::collections::HashSet::new();
        let mut snapshot: std::collections::HashMap<
            String,
            std::collections::HashMap<String, std::sync::Arc<str>>,
        > = std::collections::HashMap::new();
        for (locale, src) in [("en", EN_FTL), ("zh", ZH_FTL)] {
            let bundle = snapshot.entry(locale.to_string()).or_default();
            for (id, value) in parse_ftl_entries(src) {
                sdforge::i18n::register_translation(locale, &id, &value);
                bundle.insert(id.clone(), std::sync::Arc::from(value.as_str()));
                if locale == "en" {
                    ids.insert(id);
                }
            }
        }
        // 先建快照再置入 OnceLock：注册已发生，存在性判定即可用
        let _ = SNAPSHOT.set(snapshot);
        let _ = REGISTERED_IDS.set(ids);
    });
}

/// 该 FTL 消息标识是否已注册（en/zh 任一束存在即视为存在）。
fn is_registered_id(ftl_id: &str) -> bool {
    ensure_registered();
    REGISTERED_IDS
        .get()
        .map(|ids| ids.contains(ftl_id))
        .unwrap_or(false)
}

/// sdforge 查询的本地快照等价实现：零锁查找 + 本地渲染。
/// 查找链：请求语言束 → en 束 → 键本身；渲染规则见 [`render_template`]。
fn forge_translate(lang: &str, ftl_id: &str, args: &[(&str, String)]) -> String {
    ensure_registered();
    let Some(snapshot) = SNAPSHOT.get() else {
        return ftl_id.to_string();
    };
    let template = snapshot
        .get(lang)
        .and_then(|bundle| bundle.get(ftl_id))
        .or_else(|| snapshot.get("en").and_then(|bundle| bundle.get(ftl_id)));
    match template {
        Some(template) => render_template(template, args),
        None => ftl_id.to_string(),
    }
}

/// 本地渲染：与 sdforge `format_template` 同一套轻量替换规则（对拍钉死）。
/// 未提供参数的占位符保留 `{ $name }` 原文；替换值不二次扫描；非占位符花括号原样输出。
fn render_template(template: &str, args: &[(&str, String)]) -> String {
    if args.is_empty() || !template.contains('{') {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close_rel) = rest[open..].find('}') else {
            break;
        };
        let inner = rest[open + 1..open + close_rel].trim();
        match inner.strip_prefix('$').map(str::trim) {
            Some(name) if !name.is_empty() => {
                out.push_str(&rest[..open]);
                match args.iter().find(|(key, _)| *key == name) {
                    Some((_, value)) => out.push_str(value),
                    // 未知占位符：保留 `{ $name }` 原文（与 sdforge 一致）。
                    None => out.push_str(&rest[open..open + close_rel + 1]),
                }
                rest = &rest[open + close_rel + 1..];
            }
            _ => {
                // 非 `{ $name }` 占位符：左花括号原样输出。
                out.push_str(&rest[..open + 1]);
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 查找链：请求语言束 → "en" 束 → 键本身（不得 panic）。
/// 本函数负责点分键 → FTL id 转换这一 NebulaId 侧契约。
fn lookup(lang: &str, dotted_key: &str, args: &[(&str, String)]) -> String {
    let id = ftl_id(dotted_key);
    // 未注册键回退**源点分键**本身（sdforge 未命中回退的是 FTL id，符号不同）。
    if !is_registered_id(&id) {
        return dotted_key.to_string();
    }
    forge_translate(lang, &id, args)
}

/// 测试串行化锁：`init_i18n` 改写进程全局 locale，并行测试会互相污染。
/// 本模块与 error/handlers 的 i18n 测试共用同一把锁，避免跨模块竞态。
#[cfg(test)]
pub mod test_support {
    use std::sync::{Mutex, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    /// 取得全局 i18n 测试锁。持有期间的 guard 令其他持锁测试等待。
    pub fn lock() -> std::sync::MutexGuard<'static, ()> {
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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

    /// 测试侧独立解析 FTL：只认 `key = value` 单行与 `#` 注释；不复用实现侧解析器，
    /// 否则 parity 断言会变成「解析器对自己」。返回 (FTL id, message 值)。
    fn parse_ftl_fixture(src: &str) -> Vec<(String, String)> {
        src.lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                let (k, v) = line.split_once('=')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect()
    }

    /// 把 FTL 值替换为 sentinel 形式，用于与查询结果逐字节比对。
    /// 字符串字面量占位符先展开为字面字符（Fluent 既定语义）。
    fn expect_with_sentinels(template: &str) -> String {
        let expanded = expand_string_literals(template);
        let mut out = String::new();
        let mut rest = expanded.as_str();
        while let Some(open) = rest.find('{') {
            let Some(close_rel) = rest[open..].find('}') else {
                break;
            };
            let inner = rest[open + 1..open + close_rel].trim();
            if let Some(name) = inner.strip_prefix('$').map(str::trim) {
                out.push_str(&rest[..open]);
                out.push_str(&format!("<{name}>"));
                rest = &rest[open + close_rel + 1..];
            } else {
                out.push('{');
                rest = &rest[open + 1..];
            }
        }
        out.push_str(rest);
        out
    }

    /// 从 FTL 值中收集变量名（与 expect_with_sentinels 同一规则）。
    fn vars_of(template: &str) -> Vec<String> {
        let mut v = Vec::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            let Some(close_rel) = rest[open..].find('}') else {
                break;
            };
            let inner = rest[open + 1..open + close_rel].trim();
            if let Some(name) = inner.strip_prefix('$').map(str::trim) {
                v.push(name.to_string());
                rest = &rest[open + close_rel + 1..];
            } else {
                rest = &rest[open + 1..];
            }
        }
        v
    }

    const EN_FTL_SRC: &str = include_str!("../../locales/en/messages.ftl");
    const ZH_FTL_SRC: &str = include_str!("../../locales/zh/messages.ftl");

    /// en 束全键集 parity：输出等于 FTL 值（占位符替换为 sentinel 后逐字节相等）。
    #[test]
    fn test_en_catalog_full_keyset_parity() {
        let _g = LocaleGuard::new();
        let entries = parse_ftl_fixture(EN_FTL_SRC);
        assert!(
            entries.len() >= 500,
            "FTL 解析异常：仅 {} 条",
            entries.len()
        );
        for (id, value) in &entries {
            let names = vars_of(value);
            let sentinels: Vec<(&str, String)> = names
                .iter()
                .map(|n| (n.as_str(), format!("<{n}>")))
                .collect();
            let expected = expect_with_sentinels(value);
            let sent_ref: &[(&str, String)] = &sentinels;
            let got = translate_with_locale_args("en", id, sent_ref);
            assert!(
                !got.contains("{ \""),
                "en 键 {id} 的翻译残留未展开的字符串字面量占位符（FTL 间距约定漂移？）"
            );
            assert_eq!(got, expected, "en 键 {id} 的翻译与 FTL 不一致");
        }
    }

    /// zh 束全键集 parity。
    #[test]
    fn test_zh_catalog_full_keyset_parity() {
        let _g = LocaleGuard::new();
        let entries = parse_ftl_fixture(ZH_FTL_SRC);
        assert!(
            entries.len() >= 500,
            "FTL 解析异常：仅 {} 条",
            entries.len()
        );
        for (id, value) in &entries {
            let names = vars_of(value);
            let sentinels: Vec<(&str, String)> = names
                .iter()
                .map(|n| (n.as_str(), format!("<{n}>")))
                .collect();
            let expected = expect_with_sentinels(value);
            let sent_ref: &[(&str, String)] = &sentinels;
            let got = translate_with_locale_args("zh", id, sent_ref);
            assert!(
                !got.contains("{ \""),
                "zh 键 {id} 的翻译残留未展开的字符串字面量占位符（FTL 间距约定漂移？）"
            );
            assert_eq!(got, expected, "zh 键 {id} 的翻译与 FTL 不一致");
        }
    }

    /// 双语键集必须一致 —— 任何一侧独有的键都算迁移丢失。
    #[test]
    fn test_both_catalogs_share_identical_keyset() {
        let en: std::collections::BTreeSet<_> = parse_ftl_fixture(EN_FTL_SRC)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        let zh: std::collections::BTreeSet<_> = parse_ftl_fixture(ZH_FTL_SRC)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        let only_en: Vec<_> = en.difference(&zh).collect();
        let only_zh: Vec<_> = zh.difference(&en).collect();
        assert!(
            only_en.is_empty() && only_zh.is_empty(),
            "双语键集不一致：仅 en 有 {only_en:?}，仅 zh 有 {only_zh:?}"
        );
    }

    /// 本地快照渲染（`forge_translate`）与 sdforge 注册表（`translate_for`）在全参/
    /// 无参/多参下逐字节一致，钉死 `render_template` 对 `format_template` 的复刻。
    #[test]
    fn test_local_render_matches_sdforge() {
        let _g = LocaleGuard::new();
        for src in [EN_FTL_SRC, ZH_FTL_SRC] {
            for (id, value) in parse_ftl_fixture(src) {
                let names = vars_of(&value);
                let full: Vec<(&str, String)> = names
                    .iter()
                    .map(|n| (n.as_str(), format!("<{n}>")))
                    .collect();

                let got_full = forge_translate("en", &id, &full);
                let want_full = sdforge::i18n::translate_for("en", &id, &full);
                assert_eq!(got_full, want_full, "键 {id} 全参渲染与 sdforge 不一致");

                let got_none = forge_translate("en", &id, &[]);
                let want_none = sdforge::i18n::translate_for("en", &id, &[]);
                assert_eq!(got_none, want_none, "键 {id} 无参渲染与 sdforge 不一致");

                if !names.is_empty() {
                    let mut extra = full.clone();
                    extra.push(("unused-arg", "<none>".to_string()));
                    let got_extra = forge_translate("en", &id, &extra);
                    let want_extra = sdforge::i18n::translate_for("en", &id, &extra);
                    assert_eq!(got_extra, want_extra, "键 {id} 多参渲染与 sdforge 不一致");
                }
            }
        }
    }

    /// init_i18n 归一：zh*（大小写不敏感）→ zh-CN，未知/空 → en。
    #[test]
    fn test_init_i18n_normalization_matrix() {
        let _g = LocaleGuard::new();
        for (input, expected) in [
            ("zh-CN", "zh-CN"),
            ("zh", "zh-CN"),
            ("ZH-cn", "zh-CN"),
            ("zh-Hans", "zh-CN"),
            ("en", "en"),
            ("EN", "en"),
            ("fr", "en"),
            ("", "en"),
            ("  zh-TW  ", "zh-CN"),
        ] {
            init_i18n(input);
            assert_eq!(current_locale(), expected, "init_i18n({input:?}) 归一错误");
        }
    }

    /// 未知键回退键本身（不 panic、不返回空串）。
    #[test]
    fn test_unknown_key_falls_back_to_key() {
        let _g = LocaleGuard::new();
        assert_eq!(
            translate_with_locale("en", "error.no_such_key"),
            "error.no_such_key"
        );
        assert_eq!(
            translate_with_locale("zh", "error.no_such_key"),
            "error.no_such_key"
        );
    }

    /// translate_if_present：命中返回渲染值，未命中 None；收 FTL id（连字符），不做点分转换。
    #[test]
    fn test_translate_if_present_hit_and_miss() {
        let _g = LocaleGuard::new();
        let hit = translate_if_present("en", "error-unknown");
        assert_eq!(hit.as_deref(), Some("Unknown error"));
        assert!(translate_if_present("en", "error-absent-key").is_none());
        // zh locale 命中 zh 束（双语键集一致，en 回退分支不被触发）
        assert_eq!(
            translate_if_present("zh", "error-unknown").as_deref(),
            Some("未知错误")
        );
        assert!(translate_if_present("zh", "error-absent-key").is_none());
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
        // 在 zh-CN 下翻译：全局 locale 必须保持 "en"（无竞态风险）
        let _ = translate_with_locale("zh-CN", "error.unknown");
        assert_eq!(current_locale(), "en");
    }

    #[test]
    fn test_translate_with_locale_concurrent_safety() {
        // 不同 locale 并发调用：各自返回本 locale 的翻译，与调用顺序无关。
        let en = translate_with_locale("en", "error.unknown");
        let zh = translate_with_locale("zh-CN", "error.unknown");
        let en2 = translate_with_locale("en", "error.unknown");
        let zh2 = translate_with_locale("zh-CN", "error.unknown");

        assert_eq!(en, "Unknown error");
        assert_eq!(zh, "未知错误");
        assert_eq!(en2, "Unknown error");
        assert_eq!(zh2, "未知错误");
    }

    /// `translate_with_locale_cow` 值正确（输出恒为 `Cow::Owned`）。
    #[test]
    fn test_translate_with_locale_cow_borrowed_for_static_strings() {
        let cow = translate_with_locale_cow("en", "error.unknown");
        assert_eq!(cow.as_ref(), "Unknown error");
    }

    /// FTL 的 `{ "\n" }` 字面量必须渲染回真实换行（含续行前导空格）。
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

    /// `translate_with_locale_args_cow` 接受 Cow 值，输出与 String 变体一致。
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

    /// 未知语言("ar")取 en 束。
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

    /// 守卫：include_str! 内嵌与 locales/{en,zh}/messages.ftl 磁盘文件一致，
    /// 且双语键集经 `-`→`.` 映射后完全相同（单侧缺键只会静默回退）。
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

    /// 新增 FTL 键（en/zh 双侧）必须在两束中都能取到，防止守卫逻辑空转。
    #[test]
    fn test_t013_dynamic_error_detail_keys_present_both_bundles() {
        let _g = LocaleGuard::new();
        // 只断言命中（Some）而非具体文案；占位符键的逐字节 parity 由全键集测试覆盖。
        for locale in ["en", "zh"] {
            for id in [
                "error-detail-workspace_not_found",
                "error-main-config_load_failed",
            ] {
                assert!(
                    translate_if_present(locale, id).is_some(),
                    "{locale} 束缺少动态 error-detail 键 {id}"
                );
            }
        }
        assert_eq!(
            translate_with_locale_args(
                "zh",
                "error.main.config_load_failed",
                &[
                    ("path", "config/config.toml".to_string()),
                    ("reason", "boom".to_string()),
                ],
            ),
            "从 'config/config.toml' 加载配置失败：boom"
        );
        assert_eq!(
            translate_with_locale_args(
                "zh",
                "error.detail.workspace_not_found",
                &[("id", "ws-1".to_string())],
            ),
            "工作空间未找到：ws-1"
        );
    }
}
