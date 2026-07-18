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

//! Internationalization (ICU i18n — Phase 8).
//!
//! Wraps `rust-i18n` to provide:
//! - Compile-time locale loading from `locales/` directory (via `i18n!` in lib.rs)
//! - Runtime locale switching via [`init_i18n`]
//! - `t!()` macro for translation lookups (re-exported from `rust_i18n`)

/// Initialize the i18n system with the given locale.
///
/// Must be called once at startup, before any `t!()` lookup that depends
/// on a non-default locale. The locale string must match a top-level key
/// in `locales/<locale>.yml` (e.g. `"en"`, `"zh-CN"`).
///
/// # Example
/// ```ignore
/// nebulaid::core::i18n::init_i18n("en");
/// ```
pub fn init_i18n(locale: &str) {
    rust_i18n::set_locale(locale);
}
