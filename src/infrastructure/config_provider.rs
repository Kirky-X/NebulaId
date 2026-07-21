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

//! ConfigProvider implementation that wraps merged configuration from confers.
//!
//! This module provides a production-ready ConfigProvider that can be used
//! with AppContainer and ConfigAdapter.

use confers::interface::ConfigProvider;
use confers::types::AnnotatedValue;
use confers::{ConfigBuilder, SourceChainBuilder};
use std::collections::HashMap;

/// A ConfigProvider implementation that stores configuration values in a HashMap.
///
/// This provider is created from merged configuration sources (file, env, etc.)
/// and provides efficient key-value access for ConfigAdapter.
///
/// # Example
///
/// ```rust,ignore
/// use crate::core::infrastructure::ConfigProviderImpl;
///
/// // Create from config file
/// let provider = ConfigProviderImpl::from_file("config/config.toml")?;
///
/// // Or create from multiple sources
/// let provider = ConfigProviderImpl::builder()
///     .file("config/config.toml")
///     .env()
///     .build()?;
/// ```
pub struct ConfigProviderImpl {
    /// Flattened key-value storage for efficient access.
    values: HashMap<String, AnnotatedValue>,
}

impl ConfigProviderImpl {
    /// Create a new empty provider.
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// Create a provider from a config file.
    ///
    /// This loads a TOML configuration file and flattens it into
    /// dot-notation keys (e.g., "database.host").
    #[allow(dead_code)]
    #[allow(clippy::result_large_err)]
    pub fn from_file(path: &str) -> confers::ConfigResult<Self> {
        let builder: ConfigBuilder<AnnotatedValue> = ConfigBuilder::new();
        let annotated = builder.file(path).build_annotated()?;
        Ok(Self::from_annotated(annotated))
    }

    /// Create a provider from an AnnotatedValue.
    ///
    /// This flattens the nested configuration structure into dot-notation keys.
    pub fn from_annotated(value: AnnotatedValue) -> Self {
        let mut values = HashMap::new();
        Self::flatten(&value, "", &mut values);
        Self { values }
    }

    /// Create a new builder for constructing the provider.
    pub fn builder() -> ConfigProviderBuilder {
        ConfigProviderBuilder::new()
    }

    /// Recursively flatten nested configuration into dot-notation keys.
    fn flatten(value: &AnnotatedValue, prefix: &str, result: &mut HashMap<String, AnnotatedValue>) {
        use confers::types::ConfigValue;

        match &value.inner {
            ConfigValue::Map(map) => {
                for (key, val) in map.iter() {
                    let new_key = if prefix.is_empty() {
                        key.to_string()
                    } else {
                        format!("{}.{}", prefix, key)
                    };
                    Self::flatten(val, &new_key, result);
                }
            }
            _ => {
                if !prefix.is_empty() {
                    result.insert(prefix.to_string(), value.clone());
                }
            }
        }
    }
}

impl Default for ConfigProviderImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigProvider for ConfigProviderImpl {
    fn get_raw(&self, key: &str) -> Option<&AnnotatedValue> {
        self.values.get(key)
    }

    fn keys(&self) -> Vec<String> {
        self.values.keys().cloned().collect()
    }
}

impl std::fmt::Debug for ConfigProviderImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigProviderImpl")
            .field("key_count", &self.values.len())
            .finish()
    }
}

/// Builder for ConfigProviderImpl.
///
/// This builder allows combining multiple configuration sources
/// (files, environment variables, defaults) into a single provider.
pub struct ConfigProviderBuilder {
    files: Vec<String>,
    env_enabled: bool,
    env_prefix: Option<String>,
    defaults: HashMap<String, confers::types::ConfigValue>,
}

impl ConfigProviderBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            env_enabled: false,
            env_prefix: None,
            defaults: HashMap::new(),
        }
    }

    /// Add a configuration file.
    pub fn file(mut self, path: impl Into<String>) -> Self {
        self.files.push(path.into());
        self
    }

    /// Enable environment variable source.
    pub fn env(mut self) -> Self {
        self.env_enabled = true;
        self
    }

    /// Enable environment variable source with a prefix.
    #[allow(dead_code)]
    pub fn env_with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.env_enabled = true;
        self.env_prefix = Some(prefix.into());
        self
    }

    /// Add a default value.
    #[allow(dead_code)]
    pub fn default(mut self, key: impl Into<String>, value: confers::types::ConfigValue) -> Self {
        self.defaults.insert(key.into(), value);
        self
    }

    /// Build the ConfigProvider.
    ///
    /// This loads all configured sources and merges them together.
    #[allow(clippy::result_large_err)]
    pub fn build(self) -> confers::ConfigResult<ConfigProviderImpl> {
        let mut chain_builder = SourceChainBuilder::new();

        // Add files (lower priority first)
        for file in &self.files {
            chain_builder = chain_builder.file(file);
        }

        // Add environment variables (higher priority)
        if self.env_enabled {
            if let Some(prefix) = &self.env_prefix {
                chain_builder = chain_builder.env_with_prefix(prefix);
            } else {
                chain_builder = chain_builder.env();
            }
        }

        // Add defaults if any
        if !self.defaults.is_empty() {
            chain_builder = chain_builder.defaults(self.defaults);
        }

        let chain = chain_builder.build();
        let annotated = chain.collect()?;

        Ok(ConfigProviderImpl::from_annotated(annotated))
    }
}

impl Default for ConfigProviderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use confers::types::ConfigValue;
    use std::sync::Mutex;

    // Serialize tests that touch environment variables to prevent cross-test leakage.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn restore_env(key: &str, saved: Option<String>) {
        match saved {
            Some(val) => std::env::set_var(key, val),
            None => std::env::remove_var(key),
        }
    }

    // ===== ConfigProviderImpl::new / Default =====

    #[test]
    fn test_empty_provider() {
        let provider = ConfigProviderImpl::new();
        assert_eq!(provider.keys().len(), 0);
        assert!(provider.get_raw("any_key").is_none());
    }

    #[test]
    fn test_default_equals_new() {
        let from_default = ConfigProviderImpl::default();
        let from_new = ConfigProviderImpl::new();
        assert_eq!(from_default.keys().len(), from_new.keys().len());
        assert_eq!(from_default.keys().len(), 0);
    }

    // ===== ConfigProviderImpl::from_annotated — flatten behavior =====

    #[test]
    fn test_from_annotated_simple_non_map_value_returns_empty() {
        // A non-Map root value has an empty prefix, so it is never inserted.
        let cases = vec![
            ConfigValue::integer(42),
            ConfigValue::uint(99),
            ConfigValue::bool(true),
            ConfigValue::float(1.5),
            ConfigValue::string("hello"),
            ConfigValue::null(),
        ];
        for value in cases {
            let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(value));
            assert_eq!(
                provider.keys().len(),
                0,
                "non-Map root should produce empty provider"
            );
        }
    }

    #[test]
    fn test_from_annotated_empty_map() {
        let empty_map = ConfigValue::map(Vec::<(&str, AnnotatedValue)>::new());
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(empty_map));
        assert_eq!(provider.keys().len(), 0);
    }

    #[test]
    fn test_from_annotated_single_level_map() {
        let map = ConfigValue::map(vec![
            ("a", AnnotatedValue::from(ConfigValue::integer(1))),
            ("b", AnnotatedValue::from(ConfigValue::string("two"))),
            ("flag", AnnotatedValue::from(ConfigValue::bool(true))),
        ]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(map));

        assert_eq!(provider.keys().len(), 3);
        assert!(provider.get_raw("a").is_some());
        assert!(provider.get_raw("b").is_some());
        assert!(provider.get_raw("flag").is_some());
        assert!(provider.get_raw("a.b").is_none());
    }

    #[test]
    fn test_from_annotated_nested_map_flattens_to_dot_notation() {
        let inner_value = AnnotatedValue::from(ConfigValue::integer(42));
        let inner_map = ConfigValue::map(vec![("subkey", inner_value)]);
        let outer_map = ConfigValue::map(vec![("section", AnnotatedValue::from(inner_map))]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(outer_map));

        assert_eq!(provider.keys().len(), 1);
        let raw = provider.get_raw("section.subkey");
        assert!(
            raw.is_some(),
            "nested key should be flattened to dot notation"
        );
        // The "section" key itself should NOT be present (only leaves are stored).
        assert!(
            provider.get_raw("section").is_none(),
            "intermediate map keys should not be stored as values"
        );
    }

    #[test]
    fn test_from_annotated_multiple_levels() {
        // {a: {b: {c: 1}}} -> "a.b.c"
        let leaf = AnnotatedValue::from(ConfigValue::integer(1));
        let mid = ConfigValue::map(vec![("c", leaf)]);
        let top = ConfigValue::map(vec![("b", AnnotatedValue::from(mid))]);
        let root = ConfigValue::map(vec![("a", AnnotatedValue::from(top))]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(root));

        assert_eq!(provider.keys().len(), 1);
        assert!(provider.get_raw("a.b.c").is_some());
        assert!(provider.get_raw("a.b").is_none());
        assert!(provider.get_raw("a").is_none());
    }

    #[test]
    fn test_from_annotated_mixed_types() {
        let map = ConfigValue::map(vec![
            ("int_val", AnnotatedValue::from(ConfigValue::integer(-5))),
            ("uint_val", AnnotatedValue::from(ConfigValue::uint(100))),
            ("float_val", AnnotatedValue::from(ConfigValue::float(2.5))),
            ("bool_val", AnnotatedValue::from(ConfigValue::bool(false))),
            ("str_val", AnnotatedValue::from(ConfigValue::string("text"))),
            ("null_val", AnnotatedValue::from(ConfigValue::null())),
        ]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(map));

        assert_eq!(provider.keys().len(), 6);
        for key in [
            "int_val",
            "uint_val",
            "float_val",
            "bool_val",
            "str_val",
            "null_val",
        ] {
            assert!(
                provider.get_raw(key).is_some(),
                "key {key} should be present"
            );
        }
    }

    #[test]
    fn test_from_annotated_array_value_not_flattened() {
        // Arrays are not Maps, so they are stored as-is at their key (not indexed).
        let array_val = ConfigValue::array(vec![
            AnnotatedValue::from(ConfigValue::integer(1)),
            AnnotatedValue::from(ConfigValue::integer(2)),
        ]);
        let map = ConfigValue::map(vec![("items", AnnotatedValue::from(array_val))]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(map));

        assert_eq!(provider.keys().len(), 1);
        assert!(provider.get_raw("items").is_some());
        // Array indices should NOT be flattened into dot notation.
        assert!(provider.get_raw("items.0").is_none());
        assert!(provider.get_raw("items.1").is_none());
    }

    #[test]
    fn test_from_annotated_null_value_inserted_when_in_map() {
        // Null at root with empty prefix is dropped; Null inside a map IS stored.
        let map = ConfigValue::map(vec![("empty", AnnotatedValue::from(ConfigValue::null()))]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(map));

        assert_eq!(provider.keys().len(), 1);
        assert!(provider.get_raw("empty").is_some());
    }

    #[test]
    fn test_from_annotated_preserves_annotated_value_metadata() {
        // The stored value should be the original AnnotatedValue (with source/path/etc.).
        let map = ConfigValue::map(vec![("k", AnnotatedValue::from(ConfigValue::integer(7)))]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(map));

        let raw = provider.get_raw("k").expect("key should exist");
        assert!(matches!(raw.inner, ConfigValue::I64(7)));
    }

    // ===== ConfigProvider trait: get_raw / keys =====

    #[test]
    fn test_get_raw_returns_value_for_existing_key() {
        let map = ConfigValue::map(vec![(
            "host",
            AnnotatedValue::from(ConfigValue::string("localhost")),
        )]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(map));

        let raw = provider.get_raw("host");
        assert!(raw.is_some());
        assert!(matches!(&raw.unwrap().inner, ConfigValue::String(s) if s == "localhost"));
    }

    #[test]
    fn test_get_raw_missing_key_returns_none() {
        let map = ConfigValue::map(vec![("a", AnnotatedValue::from(ConfigValue::integer(1)))]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(map));

        assert!(provider.get_raw("nonexistent").is_none());
        assert!(provider.get_raw("").is_none());
    }

    #[test]
    fn test_keys_returns_all_keys() {
        let map = ConfigValue::map(vec![
            ("alpha", AnnotatedValue::from(ConfigValue::integer(1))),
            ("beta", AnnotatedValue::from(ConfigValue::integer(2))),
            ("gamma", AnnotatedValue::from(ConfigValue::integer(3))),
        ]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(map));

        let mut keys = provider.keys();
        keys.sort();
        assert_eq!(
            keys,
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
    }

    #[test]
    fn test_keys_empty_provider() {
        let provider = ConfigProviderImpl::new();
        let keys = provider.keys();
        assert!(keys.is_empty());
    }

    #[test]
    fn test_provider_trait_get_raw_via_dyn_ref() {
        // Verify the ConfigProvider trait is usable through a trait object.
        let map = ConfigValue::map(vec![("x", AnnotatedValue::from(ConfigValue::integer(99)))]);
        let provider: Box<dyn ConfigProvider> = Box::new(ConfigProviderImpl::from_annotated(
            AnnotatedValue::from(map),
        ));

        assert!(provider.get_raw("x").is_some());
        assert!(provider.get_raw("y").is_none());
        assert_eq!(provider.keys().len(), 1);
    }

    // ===== Debug impl =====

    #[test]
    fn test_provider_impl_debug_does_not_panic() {
        let provider = ConfigProviderImpl::new();
        let s = format!("{:?}", provider);
        assert!(s.contains("ConfigProviderImpl"));
    }

    #[test]
    fn test_provider_impl_debug_reports_key_count() {
        let map = ConfigValue::map(vec![
            ("a", AnnotatedValue::from(ConfigValue::integer(1))),
            ("b", AnnotatedValue::from(ConfigValue::integer(2))),
        ]);
        let provider = ConfigProviderImpl::from_annotated(AnnotatedValue::from(map));
        let s = format!("{:?}", provider);
        assert!(s.contains("key_count"));
        assert!(s.contains("2"));
    }

    // ===== from_file error path =====

    #[test]
    fn test_from_file_nonexistent_path_returns_error() {
        let result = ConfigProviderImpl::from_file("definitely_does_not_exist_xxx.toml");
        assert!(
            result.is_err(),
            "loading a nonexistent file should return an error"
        );
    }

    // ===== ConfigProviderBuilder =====

    #[test]
    fn test_builder_new_returns_empty_builder() {
        // An empty builder should build successfully into an empty provider.
        let provider = ConfigProviderBuilder::new().build();
        assert!(provider.is_ok(), "empty builder should build successfully");
        assert_eq!(provider.unwrap().keys().len(), 0);
    }

    #[test]
    fn test_builder_default_equals_new() {
        // Use fully-qualified syntax because the inherent `default(self, key, value)`
        // method shadows the `Default::default()` trait method.
        let from_default: ConfigProviderBuilder = <ConfigProviderBuilder as Default>::default();
        let from_new = ConfigProviderBuilder::new();

        // Both should produce equivalent empty providers.
        let p_default = from_default.build().expect("default builder builds");
        let p_new = from_new.build().expect("new builder builds");
        assert_eq!(p_default.keys().len(), 0);
        assert_eq!(p_new.keys().len(), 0);
    }

    #[test]
    fn test_builder_file_adds_file_path() {
        // Adding a nonexistent file path should cause build() to fail,
        // proving that file() actually registered the path.
        let result = ConfigProviderBuilder::new()
            .file("definitely_does_not_exist_xxx.toml")
            .build();
        assert!(
            result.is_err(),
            "build with nonexistent file should fail, proving file() registered the path"
        );
    }

    #[test]
    fn test_builder_env_enables_env_source() {
        // Enabling env source should still allow build to succeed (env may be empty).
        let _guard = ENV_LOCK.lock().unwrap();
        let result = ConfigProviderBuilder::new().env().build();
        assert!(
            result.is_ok(),
            "builder with env() should build successfully"
        );
    }

    #[test]
    fn test_builder_env_with_prefix_scopes_vars() {
        // env_with_prefix should build successfully even when no matching vars exist.
        let _guard = ENV_LOCK.lock().unwrap();
        let result = ConfigProviderBuilder::new()
            .env_with_prefix("NEBULA_TEST_PREFIX_UNUSED")
            .build();
        assert!(
            result.is_ok(),
            "builder with env_with_prefix should build successfully"
        );
    }

    #[test]
    fn test_builder_env_with_prefix_picks_up_matching_var() {
        // Verify env_with_prefix actually loads matching env vars into the provider.
        let _guard = ENV_LOCK.lock().unwrap();
        let key = "NEBULA_TEST_PREFIX_UNUSED_HOST";
        let saved = std::env::var(key).ok();
        std::env::set_var(key, "example.com");

        let result = ConfigProviderBuilder::new()
            .env_with_prefix("NEBULA_TEST_PREFIX_UNUSED")
            .build();

        restore_env(key, saved);

        let provider = result.expect("build should succeed");
        // confers normalizes env var names to lowercase and uses dot separation,
        // so NEBULA_TEST_PREFIX_UNUSED_HOST becomes "host" under the prefix.
        // We just verify that *some* key was picked up.
        assert!(
            !provider.keys().is_empty(),
            "env var with matching prefix should be loaded: keys = {:?}",
            provider.keys()
        );
    }

    #[test]
    fn test_builder_default_adds_value() {
        // The `default(key, value)` method adds a default value that becomes
        // part of the merged configuration.
        let provider = ConfigProviderBuilder::new()
            .default("test.key", ConfigValue::string("default_value"))
            .build()
            .expect("build with default should succeed");

        let raw = provider.get_raw("test.key");
        assert!(
            raw.is_some(),
            "default value should be present in built provider"
        );
        assert!(
            matches!(&raw.unwrap().inner, ConfigValue::String(s) if s == "default_value"),
            "default value content should match"
        );
    }

    #[test]
    fn test_builder_build_with_multiple_defaults() {
        let provider = ConfigProviderBuilder::new()
            .default("a", ConfigValue::integer(1))
            .default("b", ConfigValue::bool(true))
            .default("c", ConfigValue::float(2.5))
            .build()
            .expect("build should succeed");

        assert_eq!(provider.keys().len(), 3);
        assert!(provider.get_raw("a").is_some());
        assert!(provider.get_raw("b").is_some());
        assert!(provider.get_raw("c").is_some());
    }

    #[test]
    fn test_builder_build_empty_succeeds() {
        // Building with no sources at all should succeed and produce an empty provider.
        let provider = ConfigProviderBuilder::new()
            .build()
            .expect("empty builder should succeed");
        assert_eq!(provider.keys().len(), 0);
    }

    #[test]
    fn test_builder_chain_methods_return_self() {
        // The builder methods consume and return Self, enabling chaining.
        // This test verifies that chaining compiles and the final build succeeds
        // with the expected default value present.
        let provider = ConfigProviderBuilder::new()
            .file("nonexistent.toml") // would fail alone, but defaults still apply? No—file error fails build.
            ;
        // Don't build with the bad file; instead verify chaining on a clean builder.
        let _ = provider;

        let provider = ConfigProviderBuilder::new()
            .env()
            .default("chained", ConfigValue::integer(123))
            .build()
            .expect("chained builder should build");
        assert!(provider.get_raw("chained").is_some());
    }

    #[test]
    fn test_builder_defaults_override_empty_when_no_other_source() {
        // With only defaults set, the built provider should contain exactly those keys.
        let provider = ConfigProviderBuilder::new()
            .default("only_default", ConfigValue::integer(42))
            .build()
            .expect("build should succeed");

        let keys = provider.keys();
        assert_eq!(keys.len(), 1, "expected exactly one key, got {keys:?}");
        assert!(provider.get_raw("only_default").is_some());
    }

    #[test]
    fn test_builder_debug_impl_does_not_panic() {
        // ConfigProviderBuilder does not implement Debug manually, but its
        // fields are simple types. This test verifies that we can at least
        // observe builder state via the build outcome (rather than Debug).
        let builder =
            ConfigProviderBuilder::new().default("debug_test", ConfigValue::string("value"));
        let provider = builder.build().expect("builder with default should build");
        assert!(provider.get_raw("debug_test").is_some());
    }
}
