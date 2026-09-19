// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! SDK 纯算法嵌入示例（Kit 范式，）。
//!
//! 零数据库、零网络：仅使用 Snowflake / UuidV8 纯算法。默认算法改为
//! `snowflake`（`Config::default()` 的 `segment` 需要数据库，未注入仓储时
//! 会被 SDK 以 `ConfigurationError` 拒绝）。
//!
//! 运行：
//! ```bash
//! cargo run --package nebulaid --example embedded --features sdk
//! ```

use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder;

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // 纯算法嵌入：默认算法设为 snowflake，避开需要数据库的 segment。
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;

    println!("Nebula ID embedded SDK example (zero DB / zero network)");
    println!("-- snowflake --");
    for _ in 0..5 {
        let id = generator.generate("embedded", "demo", "order").await?;
        println!("{id}");
    }

    println!("-- uuid_v8 --");
    for _ in 0..5 {
        let id = generator
            .generate_with_algorithm(AlgorithmType::UuidV8, "embedded", "demo", "trace")
            .await?;
        println!("{id}");
    }

    kit.shutdown().await;
    Ok(())
}
