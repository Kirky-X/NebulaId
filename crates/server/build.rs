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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if pre-generated proto files exist
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let manifest_path = std::path::Path::new(&manifest_dir);
    let proto_src_dir = manifest_path.join("src/proto");
    let generated_mod = proto_src_dir.join("mod.rs");

    if generated_mod.exists() {
        println!("cargo:warning=Using pre-generated proto files");
        println!("cargo:rerun-if-changed=src/proto/");
        return Ok(());
    }

    // Try to compile proto files if protoc is available
    match protoc_bin_vendored::protoc_bin_path() {
        Ok(protoc_path) => {
            std::env::set_var("PROTOC", protoc_path.clone());
            println!("cargo:warning=Using vendored protoc at: {:?}", protoc_path);

            // Go up two levels from crates/server to project root
            let project_root = manifest_path.parent().unwrap().parent().unwrap();
            let proto_dir = project_root.join("protos");
            let proto_path = proto_dir.join("nebula_id.proto");

            println!("cargo:warning=Compiling proto: {:?}", proto_path);
            println!("cargo:warning=Proto include dir: {:?}", proto_dir);

            // Ensure the proto file exists
            if !proto_path.exists() {
                return Err(format!("Proto file not found at {:?}", proto_path).into());
            }

            tonic_prost_build::compile_protos(&proto_path)?;
            println!("cargo:rerun-if-changed={}", proto_path.display());
        }
        Err(e) => {
            println!(
                "cargo:warning=protoc-bin-vendored failed ({}), skipping proto compilation",
                e
            );
            println!("cargo:warning=Pre-generated proto files will be used instead");
        }
    }

    Ok(())
}
