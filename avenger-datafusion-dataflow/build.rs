fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto");
    prost_build::Config::new()
        .boxed(".avenger.dataflow.Node.program.plan")
        .protoc_executable(protoc_bin_vendored::protoc_bin_path()?)
        .extern_path(".datafusion", "::datafusion_proto::protobuf")
        .extern_path(".datafusion_common", "::datafusion_proto_common")
        .compile_protos(&["proto/dataflow.proto"], &["proto"])?;
    Ok(())
}
