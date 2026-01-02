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

mod multi_level_cache;
pub(crate) mod redis_cache;
pub(crate) mod ring_buffer;

#[cfg(test)]
mod redis_integration_test;

// Public API - expose MultiLevelCache and cache backend trait to external consumers
pub use multi_level_cache::MultiLevelCache;
pub use multi_level_cache::{CacheBackend, CacheMetrics};
pub use redis_cache::RedisCacheBackend;

// Internal re-exports (pub(crate) so they're available within the crate but not externally)
pub(crate) use ring_buffer::RingBuffer;
