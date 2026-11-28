//! Generates the prost messages of package `avenger.transforms.v1` from
//! `proto/avenger/transforms/v1/plan.proto` with the vendored `protoc`, so that a build needs no
//! system `protoc`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto");
    prost_build::Config::new()
        .protoc_executable(protoc_bin_vendored::protoc_bin_path()?)
        .compile_protos(&["proto/avenger/transforms/v1/plan.proto"], &["proto"])?;
    Ok(())
}
