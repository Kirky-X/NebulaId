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

use confers::traits::ConfigProvider;
use confers::value::AnnotatedValue;
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
/// use nebula_core::infrastructure::ConfigProviderImpl;
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
        use confers::value::ConfigValue;

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
    defaults: HashMap<String, confers::value::ConfigValue>,
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
    pub fn env_with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.env_enabled = true;
        self.env_prefix = Some(prefix.into());
        self
    }

    /// Add a default value.
    pub fn default(mut self, key: impl Into<String>, value: confers::value::ConfigValue) -> Self {
        self.defaults.insert(key.into(), value);
        self
    }

    /// Build the ConfigProvider.
    ///
    /// This loads all configured sources and merges them together.
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

    #[test]
    fn test_empty_provider() {
        let provider = ConfigProviderImpl::new();
        assert_eq!(provider.keys().len(), 0);
        assert!(provider.get_raw("any_key").is_none());
    }
}
