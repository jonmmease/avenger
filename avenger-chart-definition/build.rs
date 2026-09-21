fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/chart.proto");
    println!("cargo:rerun-if-changed=../avenger-datafusion-dataflow/proto");
    prost_build::Config::new()
        .boxed(".avenger.chart.Mark.encoding.rect")
        .protoc_executable(protoc_bin_vendored::protoc_bin_path()?)
        .extern_path(
            ".avenger.dataflow",
            "::avenger_datafusion_dataflow::protobuf",
        )
        .extern_path(
            ".datafusion",
            "::avenger_datafusion_dataflow::protobuf::datafusion",
        )
        .extern_path(
            ".datafusion_common",
            "::avenger_datafusion_dataflow::protobuf::common",
        )
        .compile_protos(
            &["proto/chart.proto"],
            &["proto", "../avenger-datafusion-dataflow/proto"],
        )?;
    Ok(())
}
