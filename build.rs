fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        embed_manifest::embed_manifest(
            embed_manifest::new_manifest("prinstall")
                .requested_execution_level(
                    embed_manifest::manifest::ExecutionLevel::RequireAdministrator,
                ),
        )
        .expect("unable to embed manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
