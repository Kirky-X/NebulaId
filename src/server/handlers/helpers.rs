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

//! Internal error-mapping helpers shared across handler sub-modules.

use crate::core::CoreError;

/// Convert database errors to `CoreError::DatabaseError`.
pub(super) fn map_db_error<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::DatabaseError(error.to_string())
}

/// Convert UUID parse errors to `CoreError::InvalidInput`.
pub(super) fn map_uuid_error<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::InvalidInput(format!("Invalid UUID: {}", error))
}
