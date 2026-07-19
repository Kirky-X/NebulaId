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

//! Criterion micro-benchmarks for the Phase 8 T041 i18n hot paths.
//!
//! Run with: `cargo bench --bench i18n`
//!
//! Covers:
//! - `translate_with_locale` (no-args lookup, en + zh-CN)
//! - `translate_with_locale_args` (single-arg lookup, en)
//! - `CoreError::to_localized_string` (delegates to `i18n_key()` +
//!   `i18n_args()` + `translate_with_locale_args_cow`)
//! - `parse_accept_language` (locale negotiation from raw header)
//!
//! Phase 8 T041 (LOW L4 perf fix) — establishes a baseline so future
//! i18n changes can be quantified. The bench intentionally avoids
//! `set_locale` (global state) and exercises only the per-call
//! `translate_with_locale*` APIs which are concurrency-safe.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nebulaid::core::i18n::{translate_with_locale, translate_with_locale_args};
use nebulaid::core::types::CoreError;

fn bench_translate_with_locale(c: &mut Criterion) {
    c.bench_function("translate_with_locale/en/no_args", |b| {
        b.iter(|| {
            black_box(translate_with_locale(
                black_box("en"),
                black_box("error.unknown"),
            ))
        })
    });
    c.bench_function("translate_with_locale/zh-CN/no_args", |b| {
        b.iter(|| {
            black_box(translate_with_locale(
                black_box("zh-CN"),
                black_box("error.unknown"),
            ))
        })
    });
    c.bench_function("translate_with_locale/fr_fallback/no_args", |b| {
        // Unsupported locale — exercises the fallback path.
        b.iter(|| {
            black_box(translate_with_locale(
                black_box("fr"),
                black_box("error.unknown"),
            ))
        })
    });
}

fn bench_translate_with_locale_args(c: &mut Criterion) {
    c.bench_function("translate_with_locale_args/en/1_arg", |b| {
        b.iter(|| {
            let args = [("value", "test_value".to_string())];
            black_box(translate_with_locale_args(
                black_box("en"),
                black_box("error.invalid_id_format"),
                black_box(&args),
            ))
        })
    });
    c.bench_function("translate_with_locale_args/zh-CN/1_arg", |b| {
        b.iter(|| {
            let args = [("value", "test_value".to_string())];
            black_box(translate_with_locale_args(
                black_box("zh-CN"),
                black_box("error.invalid_id_format"),
                black_box(&args),
            ))
        })
    });
}

fn bench_to_localized_string(c: &mut Criterion) {
    let err_string = CoreError::InvalidIdFormat("test".to_string());
    let err_no_arg = CoreError::RateLimitExceeded;
    let err_named = CoreError::ClockMovedBackward {
        last_timestamp: 123,
    };

    c.bench_function("to_localized_string/InvalidIdFormat/en", |b| {
        b.iter(|| black_box(err_string.to_localized_string(black_box("en"))))
    });
    c.bench_function("to_localized_string/InvalidIdFormat/zh-CN", |b| {
        b.iter(|| black_box(err_string.to_localized_string(black_box("zh-CN"))))
    });
    c.bench_function("to_localized_string/RateLimitExceeded/en", |b| {
        b.iter(|| black_box(err_no_arg.to_localized_string(black_box("en"))))
    });
    c.bench_function("to_localized_string/ClockMovedBackward/en", |b| {
        b.iter(|| black_box(err_named.to_localized_string(black_box("en"))))
    });
}

fn bench_parse_accept_language(c: &mut Criterion) {
    // Typical browser header (2 candidates).
    c.bench_function("parse_accept_language/typical_2", |b| {
        b.iter(|| {
            black_box(nebulaid::server::middleware::locale::negotiate_locale_str(
                black_box("en-US,en;q=0.9"),
            ))
        })
    });
    // Longer header (5 candidates with q-values).
    c.bench_function("parse_accept_language/longer_5", |b| {
        b.iter(|| {
            black_box(nebulaid::server::middleware::locale::negotiate_locale_str(
                black_box("zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7,fr;q=0.5"),
            ))
        })
    });
    // Unsupported-only header (exercises the fallback path).
    c.bench_function("parse_accept_language/unsupported_only", |b| {
        b.iter(|| {
            black_box(nebulaid::server::middleware::locale::negotiate_locale_str(
                black_box("fr,ja,de-DE"),
            ))
        })
    });
}

criterion_group!(
    benches,
    bench_translate_with_locale,
    bench_translate_with_locale_args,
    bench_to_localized_string,
    bench_parse_accept_language,
);
criterion_main!(benches);
