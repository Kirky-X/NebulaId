// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Internationalization (i18n — unify-rust-i18n 基线, sdforge i18n 承载).
//!
//! 2026-10 selfdev-lib-adoption-r1: Fluent 引擎(fluent-bundle + unic-langid
//! 两个直依赖已删)收编为 sdforge i18n。FTL 资源 `locales/{en,zh}/messages.ftl`
//! 经 `include_str!` 编译期内嵌(内嵌与磁盘是同一份文件,结构上不可能漂移,见
//! 守卫测试 `test_embedded_ftl_matches_locales_dir`);扁平键集在首次查询或
//! `init_i18n` 时一次性注册进 sdforge `register_translation`,并同步构建
//! 本模块的只读快照 `SNAPSHOT`。
//!
//! 查询路径**不触碰 sdforge 的进程级注册表**(其 `REGISTRY` 是全局
//! `Mutex`,每次查询过锁且每次查找新建 `(locale, key)` 元组):键集在
//! 运行期不变,故经 OnceLock 快照本地查找 + 本地渲染(渲染规则与
//! sdforge `format_template` 一致,见 `render_template` 与
//! `test_local_render_matches_sdforge` 对拍)。
//!
//! 运行时进程级 locale 切换经 [`init_i18n`](`t!` 宏的全局 locale 来源);
//! 每请求 locale 翻译经 [`translate_with_locale`] 系列——被 HTTP handlers
//! 用于按 `Accept-Language` 协商出的 `Locale` 翻译错误消息,不触碰全局
//! locale 状态(并发安全)。
//!
//! # 键名约定(与 rust-i18n 时代的点分键兼容)
//!
//! FTL 消息标识不允许 `.`;源键 `a.b.c_d` 与 FTL id `a-b-c_d` 一一对应
//! (查找时把 `.` 替换为 `-`;两份 yml 中没有任何含 `-` 的键,映射双射,
//! 见守卫测试)。对外 API(`i18n_key()`、`t!` 调用点、FTL 键齐性测试的
//! 语义)全部沿用点分键,零调用点改动。
//!
//! # 回退语义(与 Fluent 时代行为一致)
//!
//! 请求 locale → `en` 束 → 键本身(永不 panic、永不返回空串);未知 locale
//! ("fr"/"ja"/"xx-XX"/"") 一律落到 `en` 束。

use std::borrow::Cow;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

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
    // 键集注册提前到启动期（R-i18n 的「静态键集注册在启动期完成」约束）：
    // FTL 解析与注册是一次性成本，放在 init_i18n 里就不落在首个认证/
    // 错误响应请求的热路径上。ensure_registered 幂等，重复调用无副作用。
    ensure_registered();
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
    let pairs: Vec<(&str, String)> = Vec::new();
    Cow::Owned(lookup(lang_of(locale), key, &pairs))
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
    lookup(lang_of(locale), key, args)
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
    // Cow 载荷先物化为 String 切片：查询签名收 &[(&str, String)]。
    // 代价：每参数一次分配。本路径仅错误响应使用（非每请求热路径），
    // 按现状接受；若将来进入每请求路径，应让查询签名直收 Cow。
    let pairs: Vec<(&str, String)> = args.iter().map(|(k, v)| (*k, v.to_string())).collect();
    lookup(lang_of(locale), key, &pairs)
}

/// `t!` 宏的运行时支撑:以进程默认 locale 翻译(当前语言 → en → 键本身)。
pub fn global_translate(key: &str, args: &[(&str, String)]) -> String {
    lookup(current_lang(), key, args)
}

/// OpenAPI 文档运行时本地化用的「命中才译」查询(openapi.rs 专用)。
///
/// 与 [`lookup`]「未命中回退键本身」的语义不同,这里显式区分命中与未命中:
/// `ftl_id`(FTL 消息标识,如 `post-generate-200`,不做点分转换)在请求语言束
/// (未命中再查 en 束)中存在时返回其渲染结果,不存在返回 `None`——调用方
/// 据此保留 utoipa 静态注册的英文规范串,OpenAPI 契约结构不变。
pub fn translate_if_present(locale: &str, ftl_id: &str) -> Option<String> {
    let lang = lang_of(locale);
    if !is_registered_id(ftl_id) {
        return None;
    }
    ensure_registered();
    // 命中判定用本地快照后，渲染也走本地快照（forge_translate），不再过
    // sdforge 的进程级 REGISTRY 锁——OpenAPI 路径是"每次请求重建整份文档"
    // 的数百次调用累积，且零锁快照语义已由 test_local_render_matches_sdforge
    // 全键集对拍钉死。
    let empty: &[(&str, String)] = &[];
    Some(forge_translate(lang, ftl_id, empty))
}

// ============================================================================
// 键集注册（sdforge i18n 承载：FTL 键集迁入 register_translation）
// ============================================================================

/// 已注册的 FTL 消息标识集合。`translate_if_present` 需要区分「命中」与
/// 「未命中」，而 sdforge 的 `translate_for` 在未命中时回退键本身，两者
/// 不可区分；用本集合判定存在性（双语键集一致，等价于「任一束存在」）。
static REGISTERED_IDS: OnceLock<std::collections::HashSet<String>> = OnceLock::new();

/// 注册期构建的本地只读快照：locale → (FTL id → 消息模板)。
///
/// sdforge 的注册表是进程级全局 `Mutex`（`i18n::REGISTRY`），每次查询
/// 过锁且每次查找新建 `(locale, key)` 元组；NebulaId 的键集在注册时即
/// 确定、运行期不变，故并持一份本地快照供查询路径零锁读取（`t!` 覆盖
/// 每条日志与错误响应，经注册表即全热路径全局串行化）。
static SNAPSHOT: OnceLock<
    std::collections::HashMap<String, std::collections::HashMap<String, std::sync::Arc<str>>>,
> = OnceLock::new();

/// 注册完成的幂等 latch：首次查询时经 [`ensure_registered`] 置位，之后
/// 所有注册尝试直接命中。不单独读取其值，仅用 `OnceLock` 的 get-or-init
/// 原子性保证「注册恰好发生一次」。
static REGISTERED: OnceLock<()> = OnceLock::new();

/// English (en) FTL 资源 — 编译期内嵌 locales/en/messages.ftl。
const EN_FTL: &str = include_str!("../../locales/en/messages.ftl");
/// Chinese (zh) FTL 资源 — 编译期内嵌 locales/zh/messages.ftl。
const ZH_FTL: &str = include_str!("../../locales/zh/messages.ftl");

/// 展开 Fluent 字符串字面量占位符 `{ "..." }` 为字面字符。
///
/// Fluent 的 pattern 里换行等控制字符必须写成 `{ "\u000A" }` 这样的
/// **字符串字面量占位符**（直接写裸换行会被解释为消息结束）。注册与快照
/// 存的是展开后的消息值，渲染侧只认 `{ $name }` 变量占位符
/// （与 sdforge `format_template` 同一规则），字符串字面量对它
/// 是未知文本、原样输出——故须在注册前展开，否则消息里会留下
/// `{ "\u000A" }` 字面痕迹（`test_multiline_fatal_message_preserves_newlines`
/// 钉住这一点）。
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
///
/// NebulaId 的消息集是扁平的（无多行续行、无属性、无 select 表达式——
/// `test_both_catalogs_share_identical_keyset` 与 parity 测试共同钉住这一
/// 形态），故按行切分即可。值经 [`expand_string_literals`] 展开字符串
/// 字面量后注册/入快照，`{ $name }` 占位符保持原样交由渲染。
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

/// 把双语 FTL 键集注册进 sdforge i18n（幂等，首次**查询**触发；
/// `init_i18n` 只写全局 locale 原子量，不触发注册）。
///
/// 注册的是**消息值**而非 FTL 原文：sdforge 的登记表是扁平的
/// `(locale, key) -> value`，不做 Fluent 解析；其 `format_template` 负责
/// `{ $name }` 轻量替换，语义与本项目此前经 FluentBundle 的渲染一致。
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
///
/// 查找链与 sdforge `translate_for` 一致（请求语言束 → en 束 → 键本身），
/// 渲染规则见 [`render_template`]（与 sdforge `format_template` 对拍钉死）。
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

/// 本地渲染：与 sdforge `format_template` 同一套轻量替换规则。
///
/// sdforge 未公开该函数，而每次经其注册表查询要过进程级 `Mutex`（键集
/// 运行期不变，锁是纯开销），故按同一规则本地实现，并由
/// `test_local_render_matches_sdforge` 对拍钉死：未提供参数的占位符
/// 保留 `{ $name }` 原文，替换值不再二次扫描，非占位符花括号原样输出。
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

/// 查找链:请求语言束 → "en" 束 → 键本身(不得 panic)。
///
/// 由 sdforge `translate_for` 承载同一链(locale → en → key)，此处保留
/// 本函数以固定 `ftl_id` 的点分→连字符转换这一 NebulaId 侧契约。
fn lookup(lang: &str, dotted_key: &str, args: &[(&str, String)]) -> String {
    let id = ftl_id(dotted_key);
    // 未注册键：保持既有契约，回退的是**源点分键**本身（sdforge 的
    // translate_for 在未命中时回退它收到的 key，即 FTL id，符号不同）
    if !is_registered_id(&id) {
        return dotted_key.to_string();
    }
    forge_translate(lang, &id, args)
}

/// 测试串行化锁：`init_i18n` 改写进程全局 locale，并行测试会互相污染。
///
/// 供本模块测试与 `core::types::error` / `server::handlers::helpers` 的
/// i18n 相关测试共用（它们全部经 `crate::core::i18n::test_support::lock()`
/// 引用同一把锁），避免跨模块的并行 locale 竞态。
///
/// `#[cfg(test)]`：消费方全部是 crate 内 lib 单测，发布库里不放行该面。
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

    /// 测试侧独立解析 FTL：只认 `key = value` 单行与 `#` 注释。
    /// 刻意不复用实现侧解析器 —— 否则 parity 断言会变成「解析器对自己」。
    /// 返回 (FTL id, message 值)。
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

    /// 把 FTL 值的 `{ $name }` 占位符替换为 sentinel，用于与替换后的
    /// 查询结果逐字节比对。
    ///
    /// Fluent 的字符串字面量占位符 `{ "..." }` 先展开为字面字符——那是
    /// Fluent 引擎的既定语义（`{ "\u000A" }` 即换行），不是实现细节。
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

    /// R-i18n-001：en 束全键集逐条 parity —— 每个 FTL 键在 en locale 下
    /// 的输出必须等于 FTL 值（占位符替换为 sentinel 后逐字节相等）。
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

    /// R-i18n-001：zh 束全键集逐条 parity。
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

    /// R-i18n-001：双语键集必须一致 —— 任何一侧独有的键都算迁移丢失。
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

    /// M3 修复对拍锚点：本地快照渲染路径（`forge_translate`）与 sdforge
    /// 注册表路径（`translate_for`）对全键集在全参 / 无参 / 多参三种
    /// 组合下逐字节一致——钉死 `render_template` 对 sdforge
    /// `format_template` 替换规则的复刻（未知占位符保留原文等）。
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

    /// translate_if_present：命中返回渲染值，未命中返回 None。注意本函数收 FTL id
    ///（连字符形态，不做点分转换），与 translate_with_locale 的点分键不同。
    #[test]
    fn test_translate_if_present_hit_and_miss() {
        let _g = LocaleGuard::new();
        let hit = translate_if_present("en", "error-unknown");
        assert_eq!(hit.as_deref(), Some("Unknown error"));
        assert!(translate_if_present("en", "error-absent-key").is_none());
        // zh locale 下命中 zh 束（双语键集一致，见 keyset 测试，故 en 回退
        // 分支在当前键集下不被触发）
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
        let _g = LocaleGuard::new();
        // 动态 error-detail 键必须在双语束齐备，否则 to_localized_string
        // 自身失效导致键齐性测试空转。
        // 本测试的目的是「动态 error-detail 键在双语束齐备」，故断言命中
        // （Some）而非具体文案：带占位符的键由全键集 parity 测试覆盖其
        // 逐字节一致性（那里按变量灌 sentinel）。
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
        // 占位符键的逐字节 parity 经 translate_with_locale_args 灌参验证
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
