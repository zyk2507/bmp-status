fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(
            &[
                "proto/gobgp.proto",
                "proto/attribute.proto",
                "proto/capability.proto",
                "proto/common.proto",
                "proto/extcom.proto",
                "proto/nlri.proto",
            ],
            &["proto"],
        )?;
    println!("cargo:rerun-if-changed=proto");
    Ok(())
}
