fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest_path = std::path::Path::new(&manifest_dir);
    let project_root = manifest_path.parent().unwrap().parent().unwrap();
    let proto_path = project_root.join("protos/nebula_id.proto");

    tonic_prost_build::compile_protos(&proto_path).expect("Failed to compile protos");
    println!("cargo:rerun-if-changed={}", proto_path.display());
}
